pub fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(async {
        let instance = wgpu::Instance::default();
        let adapter = match instance.request_adapter(&Default::default()).await {
            Ok(adapter) => adapter,
            Err(error) => {
                assert!(
                    std::env::var_os("LAGUI_REQUIRE_GPU").is_none(),
                    "GPU adapter required: {error}"
                );
                eprintln!("SKIP renderer growth: no GPU adapter: {error}");
                return None;
            }
        };
        Some(
            adapter
                .request_device(&Default::default())
                .await
                .expect("GPU device"),
        )
    })
}
