fn main() {
    reprise_printers::main(std::env::args_os().skip(1).collect());
}
