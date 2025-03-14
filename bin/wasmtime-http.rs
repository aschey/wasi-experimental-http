use ::tokio;
use anyhow::{bail, Error};
use structopt::StructOpt;
use wasi_experimental_http_wasmtime::HttpCtx;
use wasmtime::{Config, Engine, Func, Instance, Linker, Store, Val, ValType};
use wasmtime_wasi::*;

#[derive(Debug, StructOpt)]
#[structopt(name = "wasmtime-http")]
struct Opt {
    #[structopt(help = "The path of the WebAssembly module to run")]
    module: String,

    #[structopt(
        short = "i",
        long = "invoke",
        default_value = "_start",
        help = "The name of the function to run"
    )]
    invoke: String,

    #[structopt(
        short = "e",
        long = "env",
        value_name = "NAME=VAL",
        parse(try_from_str = parse_env_var),
        help = "Pass an environment variable to the program"
    )]
    vars: Vec<(String, String)>,

    #[structopt(
        short = "a",
        long = "allowed-host",
        help = "Host the guest module is allowed to make outbound HTTP requests to"
    )]
    allowed_hosts: Option<Vec<String>>,

    #[structopt(
        short = "c",
        long = "concurrency",
        help = "The maximum number of concurrent requests a module can make to allowed hosts"
    )]
    max_concurrency: Option<u32>,

    #[structopt(value_name = "ARGS", help = "The arguments to pass to the module")]
    module_args: Vec<String>,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Error> {
    let opt = Opt::from_args();
    let method = opt.invoke.clone();
    // println!("{:?}", opt);
    let (instance, mut store) =
        create_instance(opt.module, opt.vars, opt.allowed_hosts, opt.max_concurrency).await?;
    let func = instance
        .get_func(&mut store, method.as_str())
        .unwrap_or_else(|| panic!("cannot find function {}", method));

    invoke_func(func, opt.module_args, &mut store).await?;

    Ok(())
}

async fn create_instance(
    filename: String,
    vars: Vec<(String, String)>,
    allowed_hosts: Option<Vec<String>>,
    max_concurrent_requests: Option<u32>,
) -> Result<(Instance, Store<WasiCtx>), Error> {
    let mut config = Config::new();

    config.async_support(true);
    config.consume_fuel(true);

    let engine = Engine::new(&config).unwrap();
    let mut linker = Linker::new(&engine);

    let ctx = WasiCtxBuilder::new()
        .inherit_stdin()
        .inherit_stdout()
        .inherit_stderr()
        .envs(&vars)?
        .build();

    let mut store = Store::new(&engine, ctx);
    store.add_fuel(10000)?;
    store.out_of_fuel_async_yield(u64::MAX, 10000);

    wasmtime_wasi::tokio::add_to_linker(&mut linker, |cx| cx)?;

    // Link `wasi_experimental_http`
    let http = HttpCtx::new(allowed_hosts, max_concurrent_requests).await?;
    http.add_to_linker(&mut linker)?;

    let module = wasmtime::Module::from_file(store.engine(), filename)?;
    let instance = linker.instantiate(&mut store, &module)?;

    Ok((instance, store))
}

// Invoke function given module arguments and print results.
// Adapted from https://github.com/bytecodealliance/wasmtime/blob/main/src/commands/run.rs.
async fn invoke_func(
    func: Func,
    args: Vec<String>,
    mut store: &mut Store<WasiCtx>,
) -> Result<(), Error> {
    let ty = func.ty(&mut store);

    let mut args = args.iter();
    let mut values = Vec::new();
    for ty in ty.params() {
        let val = match args.next() {
            Some(s) => s,
            None => {
                bail!("not enough arguments for invocation")
            }
        };
        values.push(match ty {
            ValType::I32 => Val::I32(val.parse()?),
            ValType::I64 => Val::I64(val.parse()?),
            ValType::F32 => Val::F32(val.parse()?),
            ValType::F64 => Val::F64(val.parse()?),
            t => bail!("unsupported argument type {:?}", t),
        });
    }

    let mut results = vec![];
    func.call_async(&mut store, &values, &mut results).await?;
    for result in results {
        match result {
            Val::I32(i) => println!("{}", i),
            Val::I64(i) => println!("{}", i),
            Val::F32(f) => println!("{}", f32::from_bits(f)),
            Val::F64(f) => println!("{}", f64::from_bits(f)),
            Val::ExternRef(_) => println!("<externref>"),
            Val::FuncRef(_) => println!("<funcref>"),
            Val::V128(i) => println!("{}", i),
        };
    }

    Ok(())
}

fn parse_env_var(s: &str) -> Result<(String, String), Error> {
    let parts: Vec<_> = s.splitn(2, '=').collect();
    if parts.len() != 2 {
        bail!("must be of the form `key=value`");
    }
    Ok((parts[0].to_owned(), parts[1].to_owned()))
}
