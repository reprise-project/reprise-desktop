fn main() {
    reprise_polkit::main(std::env::args_os().skip(1).collect());
}
