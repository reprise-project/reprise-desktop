fn main() {
    reprise_explorer::main(std::env::args_os().skip(1).collect());
}
