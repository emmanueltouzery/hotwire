# Hotwire

Hotwire is a gtk GUI application that leverages the wireshark and tshark infrastructure to capture traffic and explore the contents
of tcpdump files, but displays the data in a more focused way than wireshark. Hotwire supports only a
few protocols (currently PostgreSQL, HTTP and HTTP2), but for these protocols it offers a high-level,
clear display of the network traffic, tailored for each specific protocol.
Hotwire can open tcpdump files or record traffic through a fifo file, therefore without requiring elevated privileges.

## The UI layout

![Main view screenshot](https://raw.githubusercontent.com/wiki/emmanueltouzery/Hotwire/pic1.png)

The main view is divided in four panes; from left to right and top to bottom:
1. The servers; Hotwire is only interested in client-server protocols, so it can group packets by server.
   We also display metadata there, like the number of remote hosts, the number of TCP sessions, and details
   depending on the protocol (host name for HTTP, database name for PGSQL);
2. The messages. In the case of HTTP, we group request & response in one single row, in the case of PGSQL
   we group query and query result in one row as well. It's possible to sort by any column. The color on the
   left highlights the TCP stream, so it's easier to track which messages are related to one another;
3. The incoming connections. These hold for the currently selected server only. We can see remote hosts and
   tcp streams, and selecting items here will filter the messages grid;
4. The message details view. Showing details about the currently selected message.

## Protocols

Currently Hotwire supports:

* HTTP
* HTTP2
* PGSQL (PostgreSQL wire protocol)

Note that for PGSQL you can often see "Unknown statement". This can happen with prepared statements,
where the statement is declared once and then reused. If the declaration is not caught in the recording,
Hotwire has no way of recovering it and it must show "Unknown statement". It can still recover result rows
and parameters (without types or column names though).

## HTTPS and HTTP2: decryption

It is possible to view encrypted traffic in Hotwire, the same as with wireshark and tshark, if you have the
encryption keys. You can recover the encryption keys from server software (for instance apache tomcat) or client
software (firefox, chrome). To recover the keys from chrome or firefox, launch them with:

    SSLKEYLOGFILE=browser_keylog.txt firefox
    
(or same with google-chrome)
More information is available [in the wireshark wiki](https://wiki.wireshark.org/TLS). 

Hotwire doesn't allow to open separately keylog files. Instead, you should use `editcap` to merge the
secrets in the pcap file and open the combined file with Hotwire:

    editcap --inject-secrets tls,/path/to/keylog.txt ~/testtls.pcap ~/outtls.pcapng

## Live traffic recording

You can also record and observe live network traffic in Hotwire. For that, Hotwire will open a FIFO, and
listen for pcap contents on that FIFO. Note that this will not work on Windows.
Then `tcpdump` can be invoked to write pcap data to the fifo, and Hotwire will capture and display the data
in real-time. That way Hotwire can display live traffic without elevated privileges.

When Hotwire is run as a linux native app, it can invoke `pkexec` to launch `tcpdump` with elevated privileges
and everything works transparently to the user. When it runs as a flatpak or under OSX for instance, Hotwire
gives to the user a `tcpdump` command-line to run with `sudo`.

## Installation

The recommended way to install the application on linux is with flatpak. For other platforms you'll have to
build from source -- using the rust toolchain. `Hotwire` requires `tshark` to be installed and in the PATH
to operate correctly, and `tcpdump` to record traffic, and on linux `pkexec` for simple recording.

To build from source: install rust and cargo, then run `cargo run --release`. The binary in `target/bin/hotwire`
can be copied anywhere, as it embeds icons and other dependencies (but not shared libraries like gtk).

![HTTP traffic](https://raw.githubusercontent.com/wiki/emmanueltouzery/Hotwire/pic2.png)

![Dark mode and SSL](https://raw.githubusercontent.com/wiki/emmanueltouzery/Hotwire/pic3.png)
