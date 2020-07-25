use ipcs_node::NodeConfig;

pub mod cli;

#[tokio::main]
async fn main() {
    std::env::set_var("RUST_LOG", "ipcs_node=trace,info");
    env_logger::init();

    let matches = cli::app().get_matches();

    if let Some(matches) = matches.subcommand_matches("node") {
        let config = NodeConfig {
            no_api: matches.is_present("no-api"),
        };
        return ipcs_node::run(config).await;
    }

    if let Some(matches) = matches.subcommand_matches("exec") {
        let method = matches.value_of("method").unwrap();
        let args = matches.values_of("args").unwrap().collect::<Vec<&str>>();
        let api = ipcs_api::IpcsApi::new("http://127.0.0.1:3030").unwrap();
        let res = api.exec(method, &args).await.unwrap();
        println!("{}", res);
        return;
    }

    cli::app().print_help().unwrap();
    println!();
}
