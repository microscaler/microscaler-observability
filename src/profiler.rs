//! Google Cloud Profiler agent: CPU flame graphs for Rust services in the Cloud console.
//!
//! Google ships Profiler agents for Go, Java, Node.js and Python only, but the API is language-neutral
//! (`cloudprofiler.googleapis.com/v2`): an agent long-polls `CreateProfile`, the server answers when it wants a
//! profile from this deployment (one instance at a time, about one profile a minute per deployment), the agent
//! collects it and uploads it with `UpdateProfile` as a gzipped pprof protobuf. This module is that agent for CPU:
//! `pprof` samples every thread at 99 Hz with `SIGPROF` for the requested duration (10 s), symbolizes in-process,
//! and a background thread uploads the result with the pod's Workload Identity token from the metadata server.
//!
//! **Opt-in twice.** The `cloud-profiler` feature compiles it in (Linux only); the process starts it only with
//! `MICROSCALER_CLOUD_PROFILER=1` (pp sets it; prod does not — `BRRTRouter` ADR 005). Without the variable nothing
//! here runs: no thread, no signal handler, no network.
//!
//! Failures never reach the host: every error is logged at `warn` (target `microscaler_observability::profiler`)
//! and retried with backoff, or after the delay the server asks for (`409 ABORTED` with a `RetryInfo`).

use std::io::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use pprof::protos::Message as _;

const ENV_ENABLE: &str = "MICROSCALER_CLOUD_PROFILER";
const API: &str = "https://cloudprofiler.googleapis.com/v2";
const METADATA: &str = "http://metadata.google.internal/computeMetadata/v1";
/// Sampling rate. 99 rather than 100 so sampling does not beat against 10 ms timers.
const FREQUENCY_HZ: i32 = 99;
/// What `CreateProfile` may hold the request open for before answering (the server's own long-poll limit is
/// about an hour; it answers sooner when it wants a profile).
const CREATE_TIMEOUT: Duration = Duration::from_secs(3_700);
const MIN_BACKOFF: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(3_600);

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
}

/// What `CreateProfile` / an error answered with.
enum Outcome {
    Profile { name: String, duration: Duration },
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
            "profileType": ["CPU"],
        });
        let url = format!("{API}/projects/{project}/profiles");
        match self.call("POST", &url, &body, CREATE_TIMEOUT)? {
            Outcome::RetryAfter(d) => Ok(Some(d)),
            Outcome::Profile { name, duration } => {
                let bytes = collect_cpu(duration, &self.stop)?;
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                let patch = serde_json::json!({ "profileBytes": encoded });
                let url = format!("{API}/{name}?updateMask=profileBytes");
                self.call("PATCH", &url, &patch, Duration::from_secs(60))?;
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
                Ok(Outcome::Profile { name, duration })
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
        let token = json
            .get("access_token")
            .and_then(serde_json::Value::as_str)
            .ok_or("token: no access_token")?
            .to_string();
        let ttl = json
            .get("expires_in")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(300)
            .saturating_sub(60);
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
    fn off_unless_asked() {
        // the variable is not set in tests
        assert!(start("svc", None).is_none() || enabled_from_env());
    }
}
