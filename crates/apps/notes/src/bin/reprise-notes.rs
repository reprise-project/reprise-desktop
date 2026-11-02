fn main() {
    reprise_notes::main(std::env::args_os().skip(1).collect());
}
