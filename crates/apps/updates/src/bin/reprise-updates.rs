fn main() {
    reprise_updates::main(std::env::args_os().skip(1).collect());
}
