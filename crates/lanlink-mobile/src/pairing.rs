//! Identity generation and two-phase pairing.
//!
//! The phone shows the confirmation code while the PC user decides:
//! `pair_start` runs the exchange in the background and `pair_poll`
//! reports progress.

use crate::events::Json;
use crate::links::{parse_addr, parse_fp};
use crate::runtime::{next_handle, runtime};
use lanlink_client::lanlink_auth::Identity;
use lanlink_client::DeviceInfo;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub fn generate_identity(label: &str) -> Result<Vec<u8>, String> {
    let id = Identity::generate(label).map_err(|e| e.to_string())?;
    Ok(id.to_bytes())
}

pub fn identity_fingerprint(id: &[u8]) -> Result<String, String> {
    let id = Identity::from_bytes(id).map_err(|e| e.to_string())?;
    Ok(id.fingerprint().to_hex())
}

struct PairJob {
    code: Mutex<Option<String>>,
    result: Mutex<Option<Result<lanlink_client::Paired, String>>>,
    /// Aborted on cancel, which drops the connection so the PC's prompt is
    /// withdrawn instead of the exchange running on unseen.
    task: Mutex<Option<tokio::task::AbortHandle>>,
}

fn pair_jobs() -> &'static Mutex<HashMap<i64, Arc<PairJob>>> {
    static JOBS: OnceLock<Mutex<HashMap<i64, Arc<PairJob>>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[allow(clippy::too_many_arguments)]
pub fn pair_start(
    identity: &[u8],
    name: &str,
    model: &str,
    host: &str,
    port: i32,
    fp_hex: &str,
    code: &str,
) -> Result<i64, String> {
    let id = Identity::from_bytes(identity).map_err(|e| e.to_string())?;
    let device = DeviceInfo {
        name: name.to_string(),
        model: model.to_string(),
        app_version: env!("CARGO_PKG_VERSION").into(),
    };
    let addr = parse_addr(host, port)?;
    let fp = parse_fp(fp_hex)?;
    let code = code.to_string();
    lanlink_client::lanlink_auth::pairing::normalize_code(&code).map_err(|e| e.to_string())?;
    let job = Arc::new(PairJob {
        code: Mutex::new(None),
        result: Mutex::new(None),
        task: Mutex::new(None),
    });
    let handle = next_handle();
    pair_jobs()
        .lock()
        .map_err(|_| "lock")?
        .insert(handle, job.clone());
    let task_job = job.clone();
    let task = runtime().spawn(async move {
        let job = task_job;
        let j = job.clone();
        let r = lanlink_client::pair(&id, &device, addr, fp, &code, move |c| {
            if let Ok(mut slot) = j.code.lock() {
                *slot = Some(c.to_string());
            }
        })
        .await
        .map_err(|e| e.to_string());
        if let Ok(mut slot) = job.result.lock() {
            *slot = Some(r);
        };
    });
    if let Ok(mut t) = job.task.lock() {
        *t = Some(task.abort_handle());
    }
    Ok(handle)
}

/// JSON: `{state:"connecting"|"confirm"|"done"|"failed", code?, hostName?,
/// fingerprint?, error?}`. A finished job is removed after it is reported.
pub fn pair_poll(job: i64) -> Result<String, String> {
    let pj = pair_jobs()
        .lock()
        .map_err(|_| "lock")?
        .get(&job)
        .cloned()
        .ok_or("pairing was cancelled")?;
    let mut j = Json::object();
    let result = pj.result.lock().map_err(|_| "lock")?.take();
    let code = pj.code.lock().map_err(|_| "lock")?.clone();
    match result {
        Some(Ok(p)) => {
            j.str("state", "done")
                .str("hostName", &p.host_name)
                .str("fingerprint", &p.host_fingerprint.to_hex())
                .str("code", &p.confirmation_code);
            pair_jobs().lock().map_err(|_| "lock")?.remove(&job);
        }
        Some(Err(e)) => {
            j.str("state", "failed").str("error", &e);
            pair_jobs().lock().map_err(|_| "lock")?.remove(&job);
        }
        None => match code {
            Some(c) => {
                j.str("state", "confirm").str("code", &c);
            }
            None => {
                j.str("state", "connecting");
            }
        },
    }
    Ok(j.finish())
}

pub fn pair_cancel(job: i64) {
    let pj = pair_jobs().lock().ok().and_then(|mut j| j.remove(&job));
    if let Some(t) = pj.and_then(|j| j.task.lock().ok().and_then(|mut t| t.take())) {
        t.abort();
    }
}
