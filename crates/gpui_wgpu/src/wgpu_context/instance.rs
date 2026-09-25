use std::sync::{Mutex, PoisonError};

/// Creates a wgpu instance. Every instance of the process is created under
/// one lock: the Vulkan loader (1.3.275 with the NVIDIA driver) calls a
/// null ICD entry point when a thread enumerates the instance extensions
/// while another thread negotiates the interface version of the same ICD.
/// Instance creation is rare, so the lock is uncontended outside tests.
#[allow(clippy::disallowed_methods)]
pub(crate) fn create_instance(descriptor: wgpu::InstanceDescriptor) -> wgpu::Instance {
    static CREATION: Mutex<()> = Mutex::new(());
    let _serialized = CREATION.lock().unwrap_or_else(PoisonError::into_inner);
    wgpu::Instance::new(descriptor)
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use std::sync::Barrier;

    use crate::WgpuContext;

    /// Instances created on several threads at once all come up. Unserialized,
    /// the Vulkan loader 1.3.275 and the NVIDIA driver ended the process with
    /// SIGSEGV in most runs. Does not catch a crash of a loader or driver
    /// that is not installed on the host that runs it.
    #[test]
    fn instances_created_on_several_threads_at_once_all_come_up() {
        const THREADS: usize = 6;
        const ROUNDS: usize = 3;
        let start = Barrier::new(THREADS);
        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                scope.spawn(|| {
                    start.wait();
                    for _ in 0..ROUNDS {
                        drop(WgpuContext::surfaceless_instance());
                    }
                });
            }
        });
    }
}
