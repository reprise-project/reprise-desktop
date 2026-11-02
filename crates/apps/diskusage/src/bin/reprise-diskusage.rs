fn main() {
    reprise_diskusage::main(std::env::args_os().skip(1).collect());
}
