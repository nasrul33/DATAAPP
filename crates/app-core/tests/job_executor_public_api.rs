use teratai_app_core::JobExecutor;

#[test]
fn job_executor_is_available_from_the_supported_crate_root() {
    fn require_send<T: Send>() {}

    require_send::<JobExecutor>();
}
