fn main() {
    reprise_calc::main(std::env::args_os().skip(1).collect());
}
