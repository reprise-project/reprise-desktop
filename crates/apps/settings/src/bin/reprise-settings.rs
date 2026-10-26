fn main() {
    reprise_settings::main(std::env::args_os().skip(1).collect());
}
