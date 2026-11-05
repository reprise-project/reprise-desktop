use super::*;

fn write(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn link(root: &Path, path: &str, to: &Path) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(to, p).unwrap();
}

#[test]
fn the_cpu_is_named_by_its_model() {
    let s = "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen 7  PRO 5850U\n";
    assert_eq!(cpu_name(s).as_deref(), Some("AMD Ryzen 7 PRO 5850U"));
    assert_eq!(cpu_name("processor\t: 0\n"), None);
}

#[test]
fn a_disk_is_active_for_the_share_of_the_time_it_had_io_in_flight() {
    let a = disk::Stat {
        read_sectors: 0,
        written_sectors: 100,
        busy_ms: 1000,
    };
    let b = disk::Stat {
        read_sectors: 2048,
        written_sectors: 100,
        busy_ms: 1250,
    };
    let r = disk_rates(a, b, 0.5);
    assert_eq!(r.active, 0.5);
    assert_eq!((r.read, r.write), (2_097_152, 0), "a MiB in half a second");
    assert_eq!(
        disk_rates(b, a, 1.0).active,
        0.0,
        "a counter going back reads as idle"
    );
}

/// A machine with an NVMe disk, an Ethernet adapter, an AMD and an Intel GPU, a virtio GPU and
/// the AMD card's sensor chip.
fn machine(root: &Path) {
    write(root, "proc/cpuinfo", "model name\t: Test CPU 9000\n");
    write(root, "sys/block/nvme0n1/size", "1000\n");
    write(root, "sys/block/nvme0n1/device/model", "Disk Model\n");
    write(
        root,
        "proc/diskstats",
        " 259 0 nvme0n1 10 0 100 5 20 0 200 7 0 1000 12 0 0 0 0\n",
    );
    write(root, "sys/class/net/enp4s0/operstate", "up\n");
    write(root, "sys/class/net/enp4s0/type", "1\n");
    write(root, "sys/class/net/enp4s0/device/uevent", "");
    write(root, "sys/class/net/enp4s0/statistics/rx_bytes", "1000\n");
    write(root, "sys/class/net/enp4s0/statistics/tx_bytes", "10\n");
    let pci = root.join("sys/bus/pci");
    std::fs::create_dir_all(&pci).unwrap();
    for (n, addr, driver) in [
        (0, "0000:07:00.0", "amdgpu"),
        (1, "0000:00:02.0", "i915"),
        (2, "0000:00:01.0", "virtio_gpu"),
    ] {
        let dev = format!("sys/devices/pci0000:00/{addr}");
        write(root, &format!("{dev}/vendor"), "0x1234\n");
        write(root, &format!("{dev}/device"), "0x0001\n");
        link(root, &format!("{dev}/subsystem"), &pci);
        let drv = root.join(format!("sys/bus/pci/drivers/{driver}"));
        std::fs::create_dir_all(&drv).unwrap();
        link(root, &format!("{dev}/driver"), &drv);
        link(
            root,
            &format!("sys/class/drm/card{n}/device"),
            &root.join(&dev),
        );
    }
    let amd = "sys/devices/pci0000:00/0000:07:00.0";
    write(root, &format!("{amd}/gpu_busy_percent"), "40\n");
    write(root, &format!("{amd}/mem_info_vram_used"), "1048576\n");
    write(root, &format!("{amd}/mem_info_vram_total"), "4194304\n");
    write(root, "sys/class/hwmon/hwmon3/name", "amdgpu\n");
    write(root, "sys/class/hwmon/hwmon3/temp1_input", "52000\n");
    link(root, "sys/class/hwmon/hwmon3/device", &root.join(amd));
    write(root, "sys/class/hwmon/hwmon1/name", "k10temp\n");
    write(root, "sys/class/hwmon/hwmon1/temp1_input", "61500\n");
    link(root, "proc/40/fd/3", Path::new("/dev/dri/renderD129"));
    link(root, "proc/40/fd/4", Path::new("/home/u/notes.txt"));
}

fn fdinfo(render_ns: u64) -> String {
    format!(
        "pos:\t0\ndrm-driver:\ti915\ndrm-client-id:\t9\ndrm-pdev:\t0000:00:02.0\n\
         drm-engine-render:\t{render_ns} ns\n"
    )
}

#[test]
fn every_device_is_read_and_a_gpu_without_a_counter_falls_back_to_fdinfo() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    machine(r);
    write(r, "proc/40/fdinfo/3", &fdinfo(0));
    let mut s = Sampler::new(r);
    let t = Instant::now();
    let first = s.sample(t, &[(40, 7)]);
    assert_eq!(first.cpu_name, "Test CPU 9000");
    assert_eq!(first.cpu_temp, Some(61.5), "k10temp is the CPU's chip");
    assert_eq!(first.disks.len(), 1);
    assert_eq!(first.disks[0].rates, None, "nothing to measure against yet");
    assert_eq!(first.disks[0].read_total, 100 * 512);
    assert_eq!(first.nets[0].adapter.kind, net::Kind::Ethernet);
    let cards: Vec<(&str, Source)> = (first.gpus.iter())
        .map(|g| (g.card.driver.as_str(), g.source))
        .collect();
    assert_eq!(
        cards,
        [
            ("amdgpu", Source::Device),
            ("i915", Source::None),
            ("virtio_gpu", Source::None)
        ]
    );
    assert_eq!(
        first.gpus[0].temp,
        Some(52.0),
        "the chip on the card's device"
    );
    assert_eq!(first.gpus[0].vram, Some((1 << 20, 4 << 20)));

    write(
        r,
        "proc/diskstats",
        " 259 0 nvme0n1 10 0 2148 5 20 0 200 7 0 1500 12 0 0 0 0\n",
    );
    write(r, "sys/class/net/enp4s0/statistics/rx_bytes", "3000\n");
    write(r, "proc/40/fdinfo/3", &fdinfo(250_000_000));
    let next = s.sample(t + Duration::from_secs(1), &[(40, 7)]);
    let d = next.disks[0].rates.unwrap();
    assert_eq!((d.active, d.read, d.write), (0.5, 2048 * 512, 0));
    assert_eq!(next.nets[0].rate, Some(Rate { down: 2000, up: 0 }));
    assert_eq!(next.gpus[1].source, Source::OwnProcesses);
    assert_eq!(next.gpus[1].busy, Some(0.25));
    assert_eq!(next.gpus[2].busy, None, "the virtio card has no usage data");
    assert_eq!(next.proc_gpu, HashMap::from([(40, 0.25)]));
}

#[test]
fn a_sleeping_gpu_is_left_asleep() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    machine(r);
    write(
        r,
        "sys/devices/pci0000:00/0000:07:00.0/power/runtime_status",
        "suspended\n",
    );
    let read = Sampler::new(r).sample(Instant::now(), &[]);
    let g = &read.gpus[0];
    assert!(g.asleep);
    assert_eq!((g.busy, g.temp), (None, None));
}

#[test]
fn devices_are_looked_for_again_only_after_the_rescan_interval() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    machine(r);
    let mut s = Sampler::new(r);
    let t = Instant::now();
    s.sample(t, &[]);
    write(r, "sys/block/sda/size", "10\n");
    write(r, "sys/block/sda/device/model", "USB stick\n");
    write(
        r,
        "proc/diskstats",
        " 259 0 nvme0n1 1 0 1 1 1 0 1 1 0 1 1\n 8 0 sda 1 0 1 1 1 0 1 1 0 1 1\n",
    );
    assert_eq!(s.sample(t + Duration::from_secs(1), &[]).disks.len(), 1);
    assert_eq!(s.sample(t + RESCAN, &[]).disks.len(), 2);
}

/// Prints what the sampler reads on this machine over a second:
/// `cargo test -p reprise-taskmgr -- --ignored --nocapture this_machine`.
#[test]
#[ignore]
fn this_machine() {
    let proc = Path::new("/proc");
    let procs = || -> Vec<(i32, u64)> {
        (crate::procfs::pids(proc).into_iter())
            .filter_map(|pid| Some((pid, crate::procfs::read_process(proc, pid)?.stat.start)))
            .collect()
    };
    let mut s = Sampler::new(Path::new("/"));
    s.proc_gpu = true;
    s.sample(Instant::now(), &procs());
    std::thread::sleep(Duration::from_secs(1));
    let mut r = s.sample(Instant::now(), &procs());
    let busiest: Vec<(i32, f64)> = {
        let mut v: Vec<(i32, f64)> = r.proc_gpu.drain().filter(|(_, u)| *u > 0.0).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        v
    };
    println!("{r:#?}\nGPU by process: {busiest:?}");
}
