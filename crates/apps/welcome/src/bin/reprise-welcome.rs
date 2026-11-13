fn main() {
    reprise_welcome::main(std::env::args_os().skip(1).collect());
}
