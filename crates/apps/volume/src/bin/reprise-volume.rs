fn main() {
    reprise_volume::main(std::env::args_os().skip(1).collect());
}
