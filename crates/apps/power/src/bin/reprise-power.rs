fn main() {
    reprise_power::main(std::env::args_os().skip(1).collect());
}
