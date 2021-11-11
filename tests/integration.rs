#[cfg(test)]
mod tests {
    use ::tokio;
    use anyhow::Error;
    use std::time::Instant;
    use wasi_experimental_http_wasmtime::HttpCtx;
    use wasmtime::*;
    use wasmtime_wasi::tokio::WasiCtxBuilder;
    use wasmtime_wasi::*;

    // We run the same test in a Tokio and non-Tokio environment
    // in order to make sure both scenarios are working.

    #[tokio::test(flavor = "multi_thread")]
    #[should_panic]
    async fn test_none_allowed() {
        setup_tests(None, None).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[should_panic]
    async fn test_async_none_allowed() {
        setup_tests(None, None).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[should_panic]
    async fn test_without_allowed_domains() {
        setup_tests(Some(vec![]), None).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[should_panic]
    async fn test_async_without_allowed_domains() {
        setup_tests(Some(vec![]), None).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_with_allowed_domains() {
        setup_tests(
            Some(vec![
                "https://api.brigade.sh".to_string(),
                "https://postman-echo.com".to_string(),
            ]),
            None,
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_async_with_allowed_domains() {
        setup_tests(
            Some(vec![
                "https://api.brigade.sh".to_string(),
                "https://postman-echo.com".to_string(),
            ]),
            None,
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[should_panic]
    async fn test_concurrent_requests_rust() {
        let module = "target/wasm32-wasi/release/simple_wasi_http_tests.wasm".to_string();
        make_concurrent_requests(module).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[should_panic]
    async fn test_async_concurrent_requests_rust() {
        let module = "target/wasm32-wasi/release/simple_wasi_http_tests.wasm".to_string();
        make_concurrent_requests(module).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[should_panic]
    async fn test_concurrent_requests_as() {
        let module = "tests/as/build/optimized.wasm".to_string();
        make_concurrent_requests(module).await;
    }

    async fn make_concurrent_requests(module: String) {
        let func = "concurrent";
        let (instance, mut store) = create_instance(
            module,
            Some(vec!["https://api.brigade.sh".to_string()]),
            Some(2),
        )
        .await
        .unwrap();
        let func = instance
            .get_func(&mut store, func)
            .unwrap_or_else(|| panic!("cannot find function {}", func));

        func.call_async(&mut store, &[], &mut []).await.unwrap();
    }

    async fn setup_tests(
        allowed_domains: Option<Vec<String>>,
        max_concurrent_requests: Option<u32>,
    ) {
        let modules = vec![
            "target/wasm32-wasi/release/simple_wasi_http_tests.wasm",
            "tests/as/build/optimized.wasm",
        ];
        let test_funcs = vec!["get", "post"];

        for module in modules {
            let (instance, store) = create_instance(
                module.to_string(),
                allowed_domains.clone(),
                max_concurrent_requests,
            )
            .await
            .unwrap();
            run_tests(&instance, store, &test_funcs).await.unwrap();
        }
    }

    /// Execute the module's `_start` function.
    async fn run_tests(
        instance: &Instance,
        mut store: Store<WasiCtx>,
        test_funcs: &[&str],
    ) -> Result<(), Error> {
        for func_name in test_funcs.iter() {
            let func = instance
                .get_func(&mut store, func_name)
                .unwrap_or_else(|| panic!("cannot find function {}", func_name));
            func.call_async(&mut store, &[], &mut []).await?;
        }

        Ok(())
    }

    /// Create a Wasmtime::Instance from a compiled module and
    /// link the WASI imports.
    async fn create_instance(
        filename: String,
        allowed_domains: Option<Vec<String>>,
        max_concurrent_requests: Option<u32>,
    ) -> Result<(Instance, Store<WasiCtx>), Error> {
        let start = Instant::now();

        let mut config = Config::new();
        config.async_support(true);
        config.consume_fuel(true);

        let engine = Engine::new(&config)?;
        let mut linker = Linker::new(&engine);

        let ctx = WasiCtxBuilder::new()
            .inherit_stdin()
            .inherit_stdout()
            .inherit_stderr()
            .build();

        let mut store = Store::new(&engine, ctx);
        store.add_fuel(10000)?;
        store.out_of_fuel_async_yield(u64::MAX, 10000);
        wasmtime_wasi::tokio::add_to_linker(&mut linker, |cx| cx)?;

        // Link `wasi_experimental_http`
        let http = HttpCtx::new(allowed_domains, max_concurrent_requests).await?;
        http.add_to_linker(&mut linker)?;

        let module = wasmtime::Module::from_file(store.engine(), filename)?;

        let instance = linker.instantiate(&mut store, &module)?;
        let duration = start.elapsed();
        println!("module instantiation time: {:#?}", duration);
        Ok((instance, store))
    }
}
