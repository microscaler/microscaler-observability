//! Google Cloud Profiler agent: CPU flame graphs for Rust services in the Cloud console.
//!
//! Google ships Profiler agents for Go, Java, Node.js and Python only, but the API is language-neutral
//! (`cloudprofiler.googleapis.com/v2`): an agent long-polls `CreateProfile`, the server answers when it wants a
//! profile from this deployment (one instance at a time, about one profile a minute per deployment), the agent
//! collects it and uploads it with `UpdateProfile` as a gzipped pprof protobuf. This module is that agent, run from a
//! background thread with the pod's Workload Identity token from the metadata server:
//!
//! - **CPU**: `pprof` samples every thread at 99 Hz with `SIGPROF` for the requested duration (10 s) and
//!   symbolizes in-process.
//! - **HEAP** (live bytes by allocating stack), offered only when the process runs jemalloc with profiling on:
//!   the service's global allocator is `tikv-jemallocator` 0.7 with its `profiling` feature, and the environment
//!   has `_RJEM_MALLOC_CONF=prof:true,prof_active:true,lg_prof_sample:19` (a sample about every 512 KiB
//!   allocated). `jemalloc_pprof` dumps jemalloc's sampled heap and symbolizes it in-process.
//!
//! **Memory.** Symbolizing goes through `backtrace`'s resolver, which caches what it loaded for good: every DWARF
//! section of the binary (inflated into the heap when the sections are compressed, as the `-debug` images ship them)
//! and `addr2line`'s parsed units, lines and functions. On a `-debug` Sesame service that was ~150 MiB of inflated
//! sections plus ~100–300 MiB of parsed tables, growing as profiles reached new code (24 Oct 2026). The agent empties
//! that cache after every profile, so a profile costs its symbolization while it runs and nothing after. CPU samples
//! go into `pprof`'s collector, which the microscaler fork allocates only while a profile runs (upstream holds
//! ~96 MiB in a static from the first profile on).
//!
//! **Opt-in twice.** The `cloud-profiler` feature compiles it in (Linux only); the process starts it only with
//! `MICROSCALER_CLOUD_PROFILER=1` (pp sets it; prod does not — `BRRTRouter` ADR 005). Without the variable nothing
//! here runs: no thread, no signal handler, no network.
//!
//! Failures never reach the host: every error is logged at `warn` (target `microscaler_observability::profiler`)
//! and retried with backoff, or after the delay the server asks for (`409 ABORTED` with a `RetryInfo`).
//!
//! **Archive (optional).** Cloud Profiler keeps profiles for 30 days and has no API to fetch one back. With
//! `PROFILER_ARCHIVE_BUCKET` set, the agent also writes every profile it collected - after the upload, whether the
//! upload worked or not - to that Cloud Storage bucket, with the same token, as
//! `profiles/<target>/<TYPE>/<YYYY-MM-DD>/<start, RFC 3339 UTC>_<version>_<pod>.pb.gz` (the gzipped pprof, as uploaded;
//! pod is `HOSTNAME`), with target, type, version, pod and duration as object metadata. Best effort: one try with a
//! 10 s timeout, never overwriting an object; a failure is logged at `warn` and counted ([`archive_counts`]), and
//! never delays or fails the profile.

use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use pprof::protos::Message as _;

const ENV_ENABLE: &str = "MICROSCALER_CLOUD_PROFILER";
/// Optional: the service account the agent acts as (`name@project.iam.gserviceaccount.com`).
const ENV_SERVICE_ACCOUNT: &str = "MICROSCALER_CLOUD_PROFILER_SERVICE_ACCOUNT";
const API: &str = "https://cloudprofiler.googleapis.com/v2";
const METADATA: &str = "http://metadata.google.internal/computeMetadata/v1";
/// Sampling rate. 99 rather than 100 so sampling does not beat against 10 ms timers.
const FREQUENCY_HZ: i32 = 99;
/// What `CreateProfile` may hold the request open for before answering (the server's own long-poll limit is
/// about an hour; it answers sooner when it wants a profile).
const CREATE_TIMEOUT: Duration = Duration::from_secs(3_700);
const MIN_BACKOFF: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(3_600);
/// Optional: a Cloud Storage bucket the agent also writes every profile to (see the module docs).
const ENV_ARCHIVE_BUCKET: &str = "PROFILER_ARCHIVE_BUCKET";
const STORAGE_UPLOAD: &str = "https://storage.googleapis.com/upload/storage/v1/b";
const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(10);

static ARCHIVED: AtomicU64 = AtomicU64::new(0);
static ARCHIVE_FAILED: AtomicU64 = AtomicU64::new(0);

/// Profiles written to the archive bucket, and archive writes that failed, since the process started.
pub fn archive_counts() -> (u64, u64) {
    (
        ARCHIVED.load(Ordering::Relaxed),
        ARCHIVE_FAILED.load(Ordering::Relaxed),
    )
}

type Res<T> = Result<T, String>;

/// Stops the agent thread when dropped (held by [`crate::ShutdownGuard`]).
pub struct ProfilerHandle {
    stop: Arc<AtomicBool>,
}

impl Drop for ProfilerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// `true` when the process asked for the agent (`MICROSCALER_CLOUD_PROFILER` = `1`, `true` or `on`).
pub fn enabled_from_env() -> bool {
    std::env::var(ENV_ENABLE)
        .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on"))
}

/// Start the agent thread for `service` (the Profiler "target"; one flame graph series per target and version).
pub fn start(service: &str, version: Option<&str>) -> Option<ProfilerHandle> {
    if !enabled_from_env() {
        return None;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let agent = Agent {
        target: service.to_string(),
        version: version.unwrap_or("unknown").to_string(),
        stop: Arc::clone(&stop),
        token: None,
        project: None,
        archive: std::env::var(ENV_ARCHIVE_BUCKET)
            .ok()
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty()),
        pod: std::env::var("HOSTNAME")
            .ok()
            .filter(|h| !h.trim().is_empty())
            .unwrap_or_else(|| "unknown".into()),
    };
    match thread::Builder::new()
        .name("obs-profiler".into())
        .spawn(move || agent.run())
    {
        Ok(_) => {
            tracing::info!(target: "microscaler_observability::profiler", service, "Cloud Profiler agent started");
            Some(ProfilerHandle { stop })
        }
        Err(e) => {
            tracing::warn!(target: "microscaler_observability::profiler", error = %e, "Cloud Profiler agent not started");
            None
        }
    }
}

struct Agent {
    target: String,
    version: String,
    stop: Arc<AtomicBool>,
    token: Option<(String, Instant)>,
    project: Option<String>,
    /// `PROFILER_ARCHIVE_BUCKET`, when set.
    archive: Option<String>,
    /// `HOSTNAME`: the pod, in archive object names.
    pod: String,
}

/// What `CreateProfile` / an error answered with.
enum Outcome {
    Profile {
        name: String,
        kind: String,
        duration: Duration,
        /// The whole profile `CreateProfile` answered: `UpdateProfile` must send it back (deployment included,
        /// else the server rejects the upload with "invalid project ID").
        profile: serde_json::Value,
    },
    RetryAfter(Duration),
}

impl Agent {
    fn run(mut self) {
        let mut backoff = MIN_BACKOFF;
        while !self.stop.load(Ordering::Relaxed) {
            let wait = match self.cycle() {
                Ok(None) => {
                    backoff = MIN_BACKOFF;
                    Duration::ZERO
                }
                Ok(Some(server_delay)) => server_delay,
                Err(e) => {
                    tracing::warn!(target: "microscaler_observability::profiler", error = %e, retry_in_s = backoff.as_secs(), "Cloud Profiler");
                    let w = backoff;
                    backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
                    w
                }
            };
            self.sleep(wait);
        }
    }

    /// One `CreateProfile` → collect → `UpdateProfile` round. `Ok(Some(d))`: the server asked us to wait `d`.
    fn cycle(&mut self) -> Res<Option<Duration>> {
        let project = self.project()?;
        let body = serde_json::json!({
            "deployment": {
                "projectId": project,
                "target": self.target,
                "labels": { "version": self.version, "language": "rust" },
            },
            "profileType": profile_types(),
        });
        let url = format!("{API}/projects/{project}/profiles");
        match self.call("POST", &url, &body, CREATE_TIMEOUT)? {
            Outcome::RetryAfter(d) => Ok(Some(d)),
            Outcome::Profile {
                name,
                kind,
                duration,
                profile,
            } => {
                let started = SystemTime::now();
                let bytes = match kind.as_str() {
                    "HEAP" => collect_heap(),
                    _ => collect_cpu(duration, &self.stop),
                };
                // Symbolization loaded the binary's debug info into backtrace's process-wide cache; nothing needs it
                // until the next profile (a minute or more away), so give it back now, whether or not this one worked.
                backtrace::clear_symbol_cache();
                let bytes = bytes?;
                let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                let patch = upload_body(profile, encoded);
                let url = format!("{API}/{name}?updateMask=profileBytes");
                let uploaded = self.call("PATCH", &url, &patch, Duration::from_secs(60));
                // keep our copy whether or not Cloud Profiler took it
                self.archive(&kind, duration, started, &bytes);
                uploaded?;
                Ok(None)
            }
        }
    }

    fn call(
        &mut self,
        method: &str,
        url: &str,
        body: &serde_json::Value,
        timeout: Duration,
    ) -> Res<Outcome> {
        let token = self.token()?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(timeout))
            .build()
            .into();
        let auth = format!("Bearer {token}");
        let resp = match method {
            "PATCH" => agent
                .patch(url)
                .header("Authorization", &auth)
                .send_json(body),
            _ => agent
                .post(url)
                .header("Authorization", &auth)
                .send_json(body),
        };
        let mut resp = resp.map_err(|e| format!("{method} {url}: {e}"))?;
        let status = resp.status().as_u16();
        let json: serde_json::Value = resp
            .body_mut()
            .read_json()
            .unwrap_or(serde_json::Value::Null);
        match status {
            200..=299 => {
                let name = json
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let duration = json
                    .get("duration")
                    .and_then(serde_json::Value::as_str)
                    .and_then(parse_duration)
                    .unwrap_or(Duration::from_secs(10));
                let kind = json
                    .get("profileType")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("CPU")
                    .to_string();
                Ok(Outcome::Profile {
                    name,
                    kind,
                    duration,
                    profile: json,
                })
            }
            // ABORTED: not this instance's turn; the error carries google.rpc.RetryInfo with the delay.
            409 => Ok(Outcome::RetryAfter(
                retry_delay(&json).unwrap_or(MIN_BACKOFF),
            )),
            401 => {
                self.token = None;
                Err(format!("{method} {url}: 401"))
            }
            _ => Err(format!(
                "{method} {url}: {status} {}",
                json.get("error")
                    .map(ToString::to_string)
                    .unwrap_or_default()
            )),
        }
    }

    /// Write `bytes` to the archive bucket, when one is set. Best effort: counted and logged, never an error.
    fn archive(&mut self, kind: &str, duration: Duration, started: SystemTime, bytes: &[u8]) {
        let Some(bucket) = self.archive.clone() else {
            return;
        };
        let micros = started
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX));
        let object = archive_object(&self.target, kind, &self.version, &self.pod, micros);
        let result = self.token().and_then(|token| {
            let meta = serde_json::json!({
                "name": object,
                "contentType": "application/octet-stream",
                "metadata": {
                    "target": self.target, "type": kind, "version": self.version, "pod": self.pod,
                    "duration": format!("{}s", duration.as_secs_f64()),
                },
            });
            let (content_type, body) = multipart_related(&meta, bytes);
            // ifGenerationMatch=0: create only, never replace an object already there
            let url = format!(
                "{STORAGE_UPLOAD}/{}/o?uploadType=multipart&ifGenerationMatch=0",
                percent_encode(&bucket)
            );
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(ARCHIVE_TIMEOUT))
                .build()
                .into();
            let resp = agent
                .post(&url)
                .header("Authorization", &format!("Bearer {token}"))
                .header("Content-Type", &content_type)
                .send(&body[..])
                .map_err(|e| format!("archive {object}: {e}"))?;
            match resp.status().as_u16() {
                200..=299 => Ok(()),
                status => Err(format!("archive {object}: {status}")),
            }
        });
        match result {
            Ok(()) => {
                ARCHIVED.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                ARCHIVE_FAILED.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(target: "microscaler_observability::profiler", error = %e, "profile archive");
            }
        }
    }

    /// The project from `GOOGLE_CLOUD_PROJECT`, else the metadata server.
    fn project(&mut self) -> Res<String> {
        if let Some(p) = &self.project {
            return Ok(p.clone());
        }
        let p = match std::env::var("GOOGLE_CLOUD_PROJECT") {
            Ok(p) if !p.trim().is_empty() => p,
            _ => metadata("project/project-id")?,
        };
        self.project = Some(p.clone());
        Ok(p)
    }

    /// A Workload Identity access token from the metadata server, reused until a minute before it expires.
    fn token(&mut self) -> Res<String> {
        if let Some((t, until)) = &self.token {
            if Instant::now() < *until {
                return Ok(t.clone());
            }
        }
        let raw = metadata("instance/service-accounts/default/token")?;
        let json: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("token: {e}"))?;
        let mut token = json
            .get("access_token")
            .and_then(serde_json::Value::as_str)
            .ok_or("token: no access_token")?
            .to_string();
        let mut ttl = json
            .get("expires_in")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(300)
            .saturating_sub(60);
        // Cloud Profiler refuses the pod's own Workload Identity principal (403 PERMISSION_DENIED with
        // roles/cloudprofiler.agent granted to it), so the agent can act as a service account instead: the pod's
        // token buys that account's token from IAM Credentials (the pod needs Service Account Token Creator on it).
        if let Ok(sa) = std::env::var(ENV_SERVICE_ACCOUNT) {
            if !sa.trim().is_empty() {
                (token, ttl) = impersonate(&token, sa.trim())?;
            }
        }
        self.token = Some((
            token.clone(),
            Instant::now()
                .checked_add(Duration::from_secs(ttl))
                .unwrap_or_else(Instant::now),
        ));
        Ok(token)
    }

    fn sleep(&self, total: Duration) {
        let end = Instant::now()
            .checked_add(total)
            .unwrap_or_else(Instant::now);
        while !self.stop.load(Ordering::Relaxed) {
            let now = Instant::now();
            if now >= end {
                return;
            }
            thread::sleep(
                end.saturating_duration_since(now)
                    .min(Duration::from_secs(1)),
            );
        }
    }
}

/// An access token for `service_account`, bought with `token` from IAM Credentials `generateAccessToken`.
/// Returns the token and its lifetime in seconds less a minute.
fn impersonate(token: &str, service_account: &str) -> Res<(String, u64)> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let url = format!(
        "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{service_account}:generateAccessToken"
    );
    let mut resp = agent
        .post(&url)
        .header("Authorization", &format!("Bearer {token}"))
        .send_json(
            serde_json::json!({ "scope": ["https://www.googleapis.com/auth/cloud-platform"] }),
        )
        .map_err(|e| format!("impersonate {service_account}: {e}"))?;
    let status = resp.status().as_u16();
    let json: serde_json::Value = resp
        .body_mut()
        .read_json()
        .unwrap_or(serde_json::Value::Null);
    if !(200..=299).contains(&status) {
        return Err(format!(
            "impersonate {service_account}: {status} {}",
            json.get("error")
                .map(ToString::to_string)
                .unwrap_or_default()
        ));
    }
    let access = json
        .get("accessToken")
        .and_then(serde_json::Value::as_str)
        .ok_or("impersonate: no accessToken")?
        .to_string();
    // expireTime is RFC 3339; IAM issues one-hour tokens, so refresh after 50 minutes rather than parse it.
    Ok((access, 3_000))
}

/// The `UpdateProfile` body: the profile `CreateProfile` handed out, with the bytes filled in. The server validates
/// the whole resource (an upload carrying only `profileBytes` fails on the empty `deployment.projectId`).
fn upload_body(mut profile: serde_json::Value, encoded: String) -> serde_json::Value {
    if let Some(obj) = profile.as_object_mut() {
        obj.insert("profileBytes".into(), serde_json::Value::String(encoded));
    } else {
        profile = serde_json::json!({ "profileBytes": encoded });
    }
    profile
}

/// `profiles/<target>/<TYPE>/<YYYY-MM-DD>/<start>_<version>_<pod>.pb.gz`, `start` as RFC 3339 UTC with
/// microseconds (`2026-11-05T09:15:16.962689Z`); a `/` or whitespace in a part becomes `-`.
fn archive_object(target: &str, kind: &str, version: &str, pod: &str, start_micros: u64) -> String {
    let clean = |s: &str| -> String {
        let s: String = s
            .chars()
            .map(|c| {
                if c == '/' || c.is_whitespace() {
                    '-'
                } else {
                    c
                }
            })
            .collect();
        if s.is_empty() {
            "unknown".into()
        } else {
            s
        }
    };
    let secs = start_micros / 1_000_000;
    let (y, m, d) = civil_from_days(i64::try_from(secs / 86_400).unwrap_or(0));
    let tod = secs % 86_400;
    format!(
        "profiles/{}/{}/{y:04}-{m:02}-{d:02}/{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:06}Z_{}_{}.pb.gz",
        clean(target),
        clean(&kind.to_ascii_uppercase()),
        tod / 3_600,
        tod % 3_600 / 60,
        tod % 60,
        start_micros % 1_000_000,
        clean(version),
        clean(pod),
    )
}

/// The proleptic Gregorian (year, month, day) of a day count since 1970-01-01 (Howard Hinnant's algorithm).
/// The day counts here come from a u64 of seconds / 86 400, far inside the range where none of this overflows, and
/// the day and month it returns are 1..=31 and 1..=12.
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Every byte outside RFC 3986's unreserved set as `%XX`.
fn percent_encode(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            o.push(b as char);
        } else {
            o.push('%');
            for nibble in [b >> 4, b & 0xF] {
                let c = char::from_digit(u32::from(nibble), 16).unwrap_or('0');
                o.push(c.to_ascii_uppercase());
            }
        }
    }
    o
}

/// A Cloud Storage multipart upload: the object's JSON metadata, then its bytes. Returns (Content-Type, body).
fn multipart_related(meta: &serde_json::Value, bytes: &[u8]) -> (String, Vec<u8>) {
    let boundary = "microscaler-profile-archive-7f3a9c";
    let mut body = Vec::with_capacity(bytes.len().saturating_add(512));
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/related; boundary={boundary}"), body)
}

fn metadata(path: &str) -> Res<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    agent
        .get(&format!("{METADATA}/{path}"))
        .header("Metadata-Flavor", "Google")
        .call()
        .map_err(|e| format!("metadata {path}: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("metadata {path}: {e}"))
}

/// The profile types this process can produce: CPU always, HEAP when jemalloc profiling is on.
fn profile_types() -> Vec<&'static str> {
    if jemalloc_pprof::PROF_CTL.is_some() {
        vec!["CPU", "HEAP"]
    } else {
        vec!["CPU"]
    }
}

/// jemalloc's sampled live heap as a gzipped, symbolized pprof protobuf.
fn collect_heap() -> Res<Vec<u8>> {
    let ctl = jemalloc_pprof::PROF_CTL
        .as_ref()
        .ok_or("heap: jemalloc profiling is off")?;
    let mut ctl = ctl.blocking_lock();
    if !ctl.activated() {
        return Err("heap: jemalloc profiling is not active".into());
    }
    ctl.dump_pprof().map_err(|e| format!("heap: {e}"))
}

/// Sample CPU for `duration` (cut short if the agent is stopped) and return the gzipped pprof protobuf.
fn collect_cpu(duration: Duration, stop: &AtomicBool) -> Res<Vec<u8>> {
    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(FREQUENCY_HZ)
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
        .map_err(|e| format!("profiler: {e}"))?;
    let end = Instant::now()
        .checked_add(duration)
        .unwrap_or_else(Instant::now);
    while Instant::now() < end && !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(200));
    }
    let profile = guard
        .report()
        .build()
        .map_err(|e| format!("report: {e}"))?
        .pprof()
        .map_err(|e| format!("pprof: {e}"))?;
    drop(guard);
    let mut raw = Vec::new();
    profile
        .encode(&mut raw)
        .map_err(|e| format!("encode: {e}"))?;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&raw).map_err(|e| format!("gzip: {e}"))?;
    gz.finish().map_err(|e| format!("gzip: {e}"))
}

/// `"10s"` / `"12.5s"` (protobuf Duration JSON).
fn parse_duration(s: &str) -> Option<Duration> {
    s.strip_suffix('s')?
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(Duration::from_secs_f64)
}

/// The `retryDelay` of a `google.rpc.RetryInfo` in an error's details.
fn retry_delay(json: &serde_json::Value) -> Option<Duration> {
    json.get("error")?
        .get("details")?
        .as_array()?
        .iter()
        .find_map(|d| d.get("retryDelay").and_then(serde_json::Value::as_str))
        .and_then(parse_duration)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn archive_objects_are_named_by_target_type_day_and_start() {
        // 2026-11-05T09:15:16.962689Z
        let t = 1_793_870_116_962_689;
        assert_eq!(
            archive_object("market", "cpu", "v0.1.184", "market-77c9b7c5c4-x2zr2", t),
            "profiles/market/CPU/2026-11-05/2026-11-05T09:15:16.962689Z_v0.1.184_market-77c9b7c5c4-x2zr2.pb.gz"
        );
        assert_eq!(
            archive_object("a/b", "HEAP", "", "p o d", 0),
            "profiles/a-b/HEAP/1970-01-01/1970-01-01T00:00:00.000000Z_unknown_p-o-d.pb.gz"
        );
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn archive_uploads_are_multipart_with_encoded_names() {
        assert_eq!(
            percent_encode("pw-runtime-pp-profiles"),
            "pw-runtime-pp-profiles"
        );
        assert_eq!(percent_encode("a/b c:1"), "a%2Fb%20c%3A1");
        let (ct, body) = multipart_related(&serde_json::json!({"name": "x"}), b"\x1f\x8bgz");
        let boundary = ct.strip_prefix("multipart/related; boundary=").unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with(&format!("--{boundary}\r\nContent-Type: application/json")));
        assert!(text.contains("{\"name\":\"x\"}"));
        assert!(body.windows(4).any(|w| w == b"\x1f\x8bgz"));
        assert!(text.ends_with(&format!("\r\n--{boundary}--\r\n")));
    }

    #[test]
    fn uploads_the_whole_profile_back() {
        let created = serde_json::json!({
            "name": "projects/p/profiles/1", "profileType": "CPU", "duration": "10s",
            "deployment": {"projectId": "pw-runtime-pp", "target": "billing"},
        });
        let body = upload_body(created, "Ynl0ZXM=".into());
        let at = |p: &str| body.pointer(p).and_then(serde_json::Value::as_str);
        assert_eq!(at("/deployment/projectId"), Some("pw-runtime-pp"));
        assert_eq!(at("/profileType"), Some("CPU"));
        assert_eq!(at("/profileBytes"), Some("Ynl0ZXM="));
    }

    #[test]
    fn parses_protobuf_durations() {
        assert_eq!(parse_duration("10s"), Some(Duration::from_secs(10)));
        assert_eq!(parse_duration("1.5s"), Some(Duration::from_millis(1_500)));
        assert_eq!(parse_duration("10"), None);
        assert_eq!(parse_duration("-1s"), None);
    }

    #[test]
    fn reads_retry_info_from_an_aborted_answer() {
        let j = serde_json::json!({"error": {"code": 409, "status": "ABORTED", "details": [
            {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "42.5s"}]}});
        assert_eq!(retry_delay(&j), Some(Duration::from_millis(42_500)));
        assert_eq!(retry_delay(&serde_json::json!({})), None);
    }

    #[test]
    fn collects_a_gzipped_cpu_profile() {
        let stop = AtomicBool::new(false);
        let spin = thread::spawn(|| {
            let end = Instant::now() + Duration::from_millis(600);
            let mut x = 0_u64;
            while Instant::now() < end {
                x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            }
            x
        });
        let gz = collect_cpu(Duration::from_millis(500), &stop).unwrap();
        spin.join().unwrap();
        assert_eq!(gz.get(..2), Some(&[0x1f, 0x8b][..]), "gzip magic");
    }

    #[test]
    fn offers_heap_only_with_jemalloc_profiling() {
        // the test binary's allocator is the system one, so jemalloc profiling is off
        assert_eq!(profile_types(), vec!["CPU"]);
        assert!(collect_heap().is_err());
    }

    #[test]
    fn off_unless_asked() {
        // the variable is not set in tests
        assert!(start("svc", None).is_none() || enabled_from_env());
    }
}
