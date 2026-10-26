fn main() {
    reprise_terminal::main(std::env::args_os().skip(1).collect());
}
