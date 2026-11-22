use super::github::{MAX_RELEASE_ASSET_BYTES, download_asset_bytes, github_client};
use crate::output::escape_controls;
use anyhow::Result;
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

pub(super) async fn release_archive(url: &str, tag: &str) -> Result<Vec<u8>> {
    let stderr = io::stderr();
    let width = stderr
        .is_terminal()
        .then_some(|| usize::from(console::Term::stderr().size().1).saturating_sub(1));
    download(
        &github_client()?,
        url,
        tag,
        stderr,
        width,
        MAX_RELEASE_ASSET_BYTES,
    )
    .await
}

async fn download(
    client: &reqwest::Client,
    url: &str,
    tag: &str,
    mut output: impl Write,
    width: Option<impl Fn() -> usize>,
    limit: u64,
) -> Result<Vec<u8>> {
    let label = format!("Downloading shine {}", escape_controls(tag));
    let Some(width) = width else {
        let _ = writeln!(output, "{label}...");
        return download_asset_bytes(client, url, limit, |_, _| {}).await;
    };

    let mut previous_width: usize = 0;
    let mut draw = |line: &str, width: usize| {
        let line = console::truncate_str(line, width, "");
        let columns = console::measure_text_width(&line);
        let padding = " ".repeat(previous_width.min(width).saturating_sub(columns));
        let _ = write!(output, "\r{line}{padding}");
        let _ = output.flush();
        previous_width = columns;
    };
    draw(&format!("{label}..."), width());

    let mut rate = TransferRate::default();
    let mut last_draw: Option<Instant> = None;
    let mut last_drawn = None;
    let mut total = None;
    let result = download_asset_bytes(client, url, limit, |received, length| {
        total = length;
        let now = Instant::now();
        rate.observe(now, received);
        if last_draw.is_none_or(|at| now.duration_since(at) >= Duration::from_millis(100)) {
            let width = width();
            draw(
                &progress_line(width, &label, received, total, rate.bytes_per_second),
                width,
            );
            last_draw = Some(now);
            last_drawn = Some(received);
        }
    })
    .await;
    if let Ok(bytes) = &result {
        let received = bytes.len() as u64;
        if last_drawn != Some(received) {
            let width = width();
            draw(
                &progress_line(width, &label, received, total, rate.bytes_per_second),
                width,
            );
        }
    }
    let _ = writeln!(output);
    result
}

#[derive(Default)]
struct TransferRate {
    sample: Option<(Instant, u64)>,
    bytes_per_second: Option<f64>,
}

impl TransferRate {
    fn observe(&mut self, now: Instant, received: u64) {
        let (start, previous) = *self.sample.get_or_insert((now, received));
        let elapsed = now.duration_since(start);
        if elapsed >= Duration::from_secs(1) {
            let current = (received - previous) as f64 / elapsed.as_secs_f64();
            self.bytes_per_second = Some(
                self.bytes_per_second
                    .map_or(current, |old| (old + current) / 2.0),
            );
            self.sample = Some((now, received));
        }
    }
}

fn progress_line(
    width: usize,
    label: &str,
    received: u64,
    total: Option<u64>,
    rate: Option<f64>,
) -> String {
    use std::fmt::Write;
    const MIB: f64 = 1024.0 * 1024.0;
    let mib = received as f64 / MIB;
    let total = total.filter(|&n| n > 0);
    let mut metrics = if let Some(total) = total {
        let total_mib = format!("{:.1}", total as f64 / MIB);
        format!(
            "{:3}% {mib:>amount_width$.1}/{total_mib} MiB",
            received * 100 / total,
            amount_width = total_mib.len(),
        )
    } else {
        format!("{mib:.1} MiB")
    };
    let base_width = metrics.len();
    if let Some(rate) = rate.filter(|&r| r > 0.0) {
        if rate >= MIB {
            let _ = write!(metrics, " {:.1} MiB/s", rate / MIB);
        } else if rate >= 1024.0 {
            let _ = write!(metrics, " {:.1} KiB/s", rate / 1024.0);
        } else {
            let _ = write!(metrics, " {rate:.0} B/s");
        }
        if let Some(total) = total {
            let secs = ((total - received) as f64 / rate).ceil() as u64;
            let _ = write!(metrics, " ETA {}:{:02}", secs / 60, secs % 60);
        }
    }
    // Leave room for the speed and ETA before the first throughput sample arrives.
    // All metric fields are ASCII; the release label may occupy multiple columns per character.
    let label_width = console::measure_text_width(label);
    if let Some(total) = total {
        const MAX_BAR: usize = 40;
        let reserved = metrics
            .len()
            .max(base_width + " 999.9 KiB/s ETA 00:00".len());
        let cells = width
            .saturating_sub(label_width + reserved + 4)
            .min(MAX_BAR);
        if cells >= 3 {
            let filled = (received * cells as u64 / total) as usize;
            return format!(
                "{label} [{}{}] {metrics}",
                "#".repeat(filled),
                "-".repeat(cells - filled)
            );
        }
    }
    // At narrow widths keep the indicators together and shorten the label first.
    let available = width.saturating_sub(metrics.len() + 1);
    if available == 0 {
        return console::truncate_str(&metrics, width, "").into_owned();
    }
    let label = console::truncate_str(label, available, "…");
    format!("{label} {metrics}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    #[test]
    fn rate_waits_for_a_sample_and_smooths_subsequent_estimates() {
        let start = Instant::now();
        let mut rate = TransferRate::default();
        rate.observe(start, 0);
        rate.observe(start + Duration::from_millis(200), 128);
        assert_eq!(rate.bytes_per_second, None);
        rate.observe(start + Duration::from_secs(1), 1024);
        assert_eq!(rate.bytes_per_second, Some(1024.0));
        rate.observe(start + Duration::from_secs(3), 7168);
        assert_eq!(rate.bytes_per_second, Some(2048.0));
    }

    #[test]
    fn eta_requires_both_a_measured_rate_and_a_known_length() {
        let mib = 1024 * 1024;
        let known = progress_line(80, "v1", mib, Some(2 * mib), Some(16384.0));
        assert!(known.ends_with(" 50% 1.0/2.0 MiB 16.0 KiB/s ETA 1:04"));
        assert_eq!(
            progress_line(80, "v1", mib, None, Some(mib as f64)),
            "v1 1.0 MiB 1.0 MiB/s"
        );
        assert!(!progress_line(80, "v1", mib, Some(mib), None).contains("ETA"));
        assert_eq!(progress_line(80, "v1", 0, Some(0), None), "v1 0.0 MiB");
    }

    #[test]
    fn bar_uses_spare_columns_and_caps_at_forty_cells() {
        let mib = 1024 * 1024;
        let label = "Downloading shine v2.12.0";
        let mut previous = 0;
        for width in [79, 99, 119, 199] {
            let render = |rate| progress_line(width, label, 3 * mib, Some(12 * mib), rate);
            let line = render(Some(mib as f64));
            let cells = |line: &str| {
                let (left, _) = line.split_once(']').unwrap();
                left.split_once('[').unwrap().1.len()
            };
            let count = cells(&line);
            assert_eq!(
                count,
                cells(&render(None)),
                "first speed sample moved the bar"
            );
            assert_eq!(line.matches('#').count(), count / 4);
            let near_end = progress_line(width, label, 11 * mib, Some(12 * mib), Some(mib as f64));
            assert_eq!(cells(&near_end), count);
            assert!(line.ends_with(" 25%  3.0/12.0 MiB 1.0 MiB/s ETA 0:09"));
            assert!(line.len() <= width);
            if width < 119 {
                assert!(count > previous && count < 40);
            } else {
                assert_eq!(count, 40);
            }
            previous = count;
        }
    }

    #[test]
    fn narrow_widths_preserve_indicators_before_the_release_label() {
        let metrics = " 50% 1.0/2.0 MiB 1.0 MiB/s ETA 0:01";
        for width in [0, 1, 10, metrics.len(), 48, 59] {
            let line = progress_line(
                width,
                "Downloading shine 测试版本",
                1048576,
                Some(2097152),
                Some(1048576.0),
            );
            assert!(console::measure_text_width(&line) <= width);
            assert!(!line.contains('['));
            if width >= metrics.len() {
                assert!(line.ends_with(metrics));
            }
        }
        let unknown = progress_line(
            24,
            "Downloading shine preview",
            1048576,
            None,
            Some(1048576.0),
        );
        assert!(
            unknown.ends_with("1.0 MiB 1.0 MiB/s") && console::measure_text_width(&unknown) <= 24
        );
    }

    async fn accept_request(listener: TcpListener) -> tokio::net::TcpStream {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut headers = BufReader::new(&mut socket).lines();
        while let Some(line) = headers.next_line().await.unwrap() {
            if line.is_empty() {
                break;
            }
        }
        socket
    }

    async fn local_response(
        response: &'static [u8],
        width: Option<usize>,
        limit: u64,
    ) -> (Result<Vec<u8>>, String) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut socket = accept_request(listener).await;
            socket.write_all(response).await.unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut output = Vec::new();
        let result = download(
            &client,
            &url,
            "v1",
            &mut output,
            width.map(|w| move || w),
            limit,
        )
        .await;
        server.await.unwrap();
        (result, String::from_utf8(output).unwrap())
    }

    #[tokio::test]
    async fn terminal_finishes_the_bar_and_redirected_output_stays_on_one_line() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nbody";
        let (result, output) = local_response(response, Some(80), 4).await;
        assert_eq!(result.unwrap(), b"body");
        assert!(output.contains("]   0%"));
        assert!(output.contains("] 100%"));
        assert!(output.ends_with('\n'));
        let (result, output) = local_response(response, None, 4).await;
        assert_eq!(result.unwrap(), b"body");
        assert_eq!(output, "Downloading shine v1...\n");
    }

    #[tokio::test]
    async fn slow_download_displays_rate_and_eta_before_the_last_chunk() {
        // The server waits for each rendered frame before releasing the next bytes.
        struct Screen {
            bytes: Vec<u8>,
            frames: tokio::sync::mpsc::UnboundedSender<String>,
        }
        impl Write for Screen {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.bytes.extend_from_slice(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                let frame = String::from_utf8(std::mem::take(&mut self.bytes)).unwrap();
                let _ = self.frames.send(frame);
                Ok(())
            }
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (frames, mut rendered) = tokio::sync::mpsc::unbounded_channel::<String>();
        let server = tokio::spawn(async move {
            let mut socket = accept_request(listener).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n")
                .await
                .unwrap();
            assert!(rendered.recv().await.unwrap().contains("Downloading"));
            assert!(rendered.recv().await.unwrap().contains(" 0%"));
            tokio::time::sleep(Duration::from_millis(1100)).await;
            socket.write_all(b"1234").await.unwrap();
            let halfway = rendered.recv().await.unwrap();
            assert!(
                halfway.contains(" 50%") && halfway.contains(" B/s ETA "),
                "{halfway}"
            );
            socket.write_all(b"5678").await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let result = download(
            &client,
            &url,
            "preview",
            Screen {
                bytes: Vec::new(),
                frames,
            },
            Some(|| 80),
            8,
        )
        .await;
        server.await.unwrap();
        assert_eq!(result.unwrap(), b"12345678");
    }

    #[tokio::test]
    async fn chunked_download_counts_decoded_bytes_and_enforces_the_limit() {
        let response =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nab\r\n2\r\ncd\r\n0\r\n\r\n";
        let (result, output) = local_response(response, Some(80), 4).await;
        assert_eq!(result.unwrap(), b"abcd");
        assert!(output.contains("0.0 MiB"));
        assert!(!output.contains('%'));
        let (result, output) = local_response(response, Some(80), 3).await;
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeding the 3-byte limit")
        );
        assert!(output.ends_with('\n'));
    }

    #[tokio::test]
    async fn oversized_or_interrupted_responses_end_the_line_without_completion() {
        for (limit, expected) in [
            (3, "reports 4 bytes"),
            (4, "failed to read release asset bytes"),
        ] {
            let (result, output) = local_response(
                b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nab",
                Some(80),
                limit,
            )
            .await;
            assert!(result.unwrap_err().to_string().contains(expected));
            assert!(output.ends_with('\n'));
            assert!(!output.contains("100%"));
        }
    }
}
