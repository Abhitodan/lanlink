use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lanlink-net")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);

/// Next opaque handle, shared by sessions and pairing jobs.
pub(crate) fn next_handle() -> i64 {
    NEXT_HANDLE.fetch_add(1, Ordering::SeqCst)
}
