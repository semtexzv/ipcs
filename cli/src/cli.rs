use clap::{App, Arg, SubCommand};

fn node_cmd() -> clap::App<'static, 'static> {
    SubCommand::with_name("node")
        .about("Runs the IPCS node")
        .version("0.0.0")
        .arg(Arg::with_name("no-api").long("no-api").short("n").help("Disable built-in HTTP API"))
}

fn exec_cmd() -> clap::App<'static, 'static> {
    SubCommand::with_name("exec")
        .about("Execute IPCS function on blocks")
        .arg(
            Arg::with_name("method")
                .help("Hash of the IPCS function to execute")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::with_name("args")
                .help("Arguments to the method (hashes of IPFS objects)")
                .required(false)
                .multiple(true)
                .index(2),
        )
}

pub fn app() -> clap::App<'static, 'static> {
    App::new("IPCS cli interface")
        .version("0.0.0")
        .author("Michal H. <semtexzv@gmail.com")
        .about("Inter-planetary computation system")
        .subcommand(node_cmd())
        .subcommand(exec_cmd())
        .setting(clap::AppSettings::SubcommandRequiredElseHelp)
}
