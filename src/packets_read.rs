use crate::config;
use crate::config::Config;
use crate::tshark_communication;
use crate::tshark_communication::TSharkPacket;
use nix::sys::signal::Signal;
use nix::unistd::Pid;
use quick_xml::events::Event;
use signal_hook::iterator::Signals;
use std::borrow::Cow;
use std::io::BufRead;
use std::io::BufReader;
#[cfg(target_family = "unix")]
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::thread;
use std::time::Duration;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum TSharkInputType {
    File,
    Fifo,
}

#[derive(Debug)]
pub enum InputStep {
    StartedTShark(Child),
    Packet(Box<TSharkPacket>), // Box due to large variant
    Eof,
}

pub type ParseInputStep = Result<InputStep, String>;

// it would be possible to ask tshark to "mix in" a keylog file
// when opening the pcap file
// (obtain the keylog file through `SSLKEYLOGFILE=browser_keylog.txt google-chrome` or firefox,
// pass it to tshark through -o ssh.keylog_file:/path/to/keylog)
// but we get in flatpak limitations (can only access the file that the user opened
// due to the sandbox) => better to just mix in the secrets manually and open a single
// file. this is done through => editcap --inject-secrets tls,/path/to/keylog.txt ~/testtls.pcap ~/outtls.pcapng
pub fn invoke_tshark(
    input_type: TSharkInputType,
    fname: &Path,
    filters: &str,
    sender: relm::Sender<ParseInputStep>,
) {
    // piping from tshark, not to load the entire JSON in ram...
    let mut tshark_params = vec![
        if input_type == TSharkInputType::File {
            "-r"
        } else {
            "-i"
        },
        fname.to_str().expect("invalid filename"),
        "-Tpdml",
        // "-o",
        // "ssl.keylog_file:/home/emmanuel/chrome_keylog.txt",
        // "tcp.stream eq 104",
    ];
    let pcap_output = config::get_tshark_pcap_output_path();
    if input_type == TSharkInputType::Fifo {
        // -l == flush after each packet
        tshark_params.extend(&["-w", pcap_output.to_str().unwrap(), "-l"]);
    } else {
        // if I filter in fifo mode then tshark doesn't write the output pcap file
        tshark_params.extend(&[filters]);
    }
    let tshark_child = Command::new("tshark")
        .args(&tshark_params)
        .stdout(Stdio::piped())
        .spawn();
    if tshark_child.is_err() {
        sender
            .send(Err(format!("Error launching tshark: {:?}", tshark_child)))
            .unwrap();
        return;
    }
    let mut tshark_child = tshark_child.unwrap();
    let buf_reader = BufReader::new(tshark_child.stdout.take().unwrap());
    sender
        .send(Ok(InputStep::StartedTShark(tshark_child)))
        .unwrap();
    parse_pdml_stream(buf_reader, sender);
}

pub fn parse_pdml_stream<B: BufRead>(buf_reader: B, sender: relm::Sender<ParseInputStep>) {
    let mut xml_reader = quick_xml::Reader::from_reader(buf_reader);
    let mut buf = vec![];
    loop {
        match xml_reader.read_event(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if e.name() == b"packet" {
                    match tshark_communication::parse_packet(&mut xml_reader) {
                        Ok(packet) => sender
                            .send(Ok(InputStep::Packet(Box::new(packet))))
                            .unwrap(),
                        Err(e) => {
                            sender
                                .send(Err(format!(
                                    "xml parsing error: {} at tshark output offset {}",
                                    e,
                                    xml_reader.buffer_position()
                                )))
                                .unwrap();
                            break;
                        }
                    }
                }
            }
            Ok(Event::Eof) => {
                sender.send(Ok(InputStep::Eof)).unwrap();
                break;
            }
            Err(e) => {
                sender
                    .send(Err(format!(
                        "xml parsing error: {} at tshark output offset {}",
                        e,
                        xml_reader.buffer_position()
                    )))
                    .unwrap();
                break;
            }
            _ => {}
        };
        buf.clear();
    }
}

#[cfg(target_family = "windows")]
pub fn cleanup_child_processes(
    tcpdump_child: Option<Child>,
    tshark_child: Option<Child>,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg(target_family = "unix")]
pub fn cleanup_child_processes(
    tcpdump_child: Option<Child>,
    tshark_child: Option<Child>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(_tcpdump_child) = tcpdump_child {
        let mut tcpdump_child = _tcpdump_child;
        // seems like we can't kill tcpdump, even though it's our child (owned by another user),
        // but it's not needed (presumably because we kill tshark, which reads from the fifo,
        // and the fifo itself)
        // if let Err(e) = tcpdump_pid.kill() {
        //     eprintln!("kill1 fails {:?}", e);
        // }

        // try_wait doesn't work, wait hangs, not doing anything leaves zombie processes
        // i found this way of regularly calling try_wait until it succeeds...
        glib::idle_add_local(move || {
            glib::Continue(
                !matches!(tcpdump_child.try_wait(), Ok(Some(s)) if s.code().is_some() || s.signal().is_some()),
            )
        });
    }
    if let Some(_tshark_child) = tshark_child {
        let mut tshark_child = _tshark_child;

        // soooooooo... if I use child.kill() then when I read from a local fifo file (mkfifo)
        // and I cancel the reading from the fifo, and nothing was written to the fifo at all,
        // we do kill the tshark process, but our read() on the pipe from tshark hangs.
        // I don't know why. However if I use nix to send a SIGINT, our read() is interrupted
        // and all is good...
        // It might be because tshark launches a child process, dumpcap, because I ask it to save
        // the pcap file. Or not... I didn't check too much.
        //
        // tshark_child.kill()?;
        nix::sys::signal::kill(
            Pid::from_raw(tshark_child.id() as libc::pid_t),
            Some(Signal::SIGINT),
        )?;

        // try_wait doesn't work, wait hangs, not doing anything leaves zombie processes
        // i found this way of regularly calling try_wait until it succeeds...
        glib::idle_add_local(move || glib::Continue(!try_wait_has_exited(&mut tshark_child)));
    }
    Ok(())
}

pub fn try_wait_has_exited(child: &mut Child) -> bool {
    matches!(child.try_wait(), Ok(Some(s)) if s.code().is_some() || s.signal().is_some())
}

pub fn get_tcpdump_params(fifo_path: &Path) -> Vec<Cow<str>> {
    let mut tcpdump_params: Vec<Cow<str>> = [
        "tcpdump",
        "-ni",
        "any",
        "-s0",
        "--immediate-mode",
        "--packet-buffered",
        "-w",
    ]
    .iter()
    .map(|s| Cow::Borrowed(*s))
    .collect();
    tcpdump_params.push(Cow::Owned(fifo_path.to_str().unwrap().to_string()));
    let tcpdump_custom_buf_size_kib_str = Config::read_config()
        .custom_tcpdump_buffer_size_kib
        .map(|b| b.to_string());
    if let Some(tcpdump_buf_str) = tcpdump_custom_buf_size_kib_str.as_ref() {
        tcpdump_params.extend(vec![
            Cow::Borrowed("-B"),
            Cow::Owned(tcpdump_buf_str.to_string()),
        ]);
    }

    tcpdump_params
}

pub fn setup_fifo_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    // i wanted to use the temp folder but I got permissions issues,
    // which I don't fully understand.
    let fifo_path = config::get_tcpdump_fifo_path();
    if !fifo_path.exists() {
        nix::unistd::mkfifo(
            &fifo_path,
            nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
        )?;
    }
    Ok(fifo_path)
}

pub fn invoke_tcpdump(fifo_path: &Path) -> Result<Child, Box<dyn std::error::Error>> {
    let tcpdump_params = get_tcpdump_params(&fifo_path);
    let mut tcpdump_child = Command::new("pkexec")
        .args(tcpdump_params.iter().map(|s| s.to_string()))
        .spawn()
        .map_err(|e| format!("Error launching pkexec: {:?}", e))?;

    // yeah sleeping 50ms in the gui thread...
    // but it's the easiest. pkexec needs some tome to init, try to launch that
    // app and fail... on my computer 50ms is consistently enough.
    std::thread::sleep(Duration::from_millis(50));
    if let Ok(Some(status)) = tcpdump_child.try_wait() {
        return Err(format!("Failed to execute tcpdump, pkexec exit code {}", status).into());
    }
    Ok(tcpdump_child)
}

pub fn register_child_process_death(sender: relm::Sender<()>) {
    thread::spawn(move || {
        const SIGNALS: &[libc::c_int] = &[signal_hook::consts::signal::SIGCHLD];
        let mut sigs = Signals::new(SIGNALS).unwrap();
        for signal in &mut sigs {
            sender.send(()).expect("send child died msg");
            if let Err(e) = signal_hook::low_level::emulate_default_handler(signal) {
                eprintln!("Error calling the low-level signal hook handling: {:?}", e);
            }
        }
    });
}
