fn main() {
    reprise_import::main(std::env::args_os().skip(1).collect());
}
