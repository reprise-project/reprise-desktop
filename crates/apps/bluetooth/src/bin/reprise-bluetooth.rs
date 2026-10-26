fn main() {
    reprise_bluetooth::main(std::env::args_os().skip(1).collect());
}
