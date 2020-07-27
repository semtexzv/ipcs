use clap::{App, Arg, SubCommand};

fn node_cmd() -> clap::App<'static, 'static> {
    SubCommand::with_name("node")
        .about("Runs the IPCS node")
        .version("0.0.0")
        .arg(Arg::with_name("no-api").long("no-api").short("n").help("Disable built-in HTTP API"))
        .arg(Arg::with_name("bootstrap-node")
            .long("bootstrap-node")
            .short("b")
            .takes_value(true)
            .help("Add node to bootstrap list")
        )
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


pub fn fun_cmd() -> clap::App<'static, 'static> {
    SubCommand::with_name("fun")
        .about("Work with functions")
        .subcommand(
            SubCommand::with_name("new"),
        )
        .subcommand(
            SubCommand::with_name("build")
        )
        .subcommand(
            SubCommand::with_name("deploy")
        )
}

pub fn app() -> clap::App<'static, 'static> {
    App::new("IPCS cli interface")
        .version("0.0.0")
        .author("Michal H. <semtexzv@gmail.com")
        .about("Inter-planetary computation system")
        .subcommand(node_cmd())
        .subcommand(exec_cmd())
        //.subcommand(fun_cmd())
        .setting(clap::AppSettings::SubcommandRequiredElseHelp)
}
