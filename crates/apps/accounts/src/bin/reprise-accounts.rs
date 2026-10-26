fn main() {
    reprise_accounts::main(std::env::args_os().skip(1).collect());
}
