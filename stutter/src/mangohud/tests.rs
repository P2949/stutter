use super::*;

const MANGOHUD_WITH_METADATA: &str = "\
os,cpu,gpu,ram,kernel,driver,cpuscheduler\n\
'Gentoo Linux',Intel Core i5-10600K CPU @ 4.10GHz,Intel(R) UHD Graphics 630 (CML GT2),32148228,7.0.1-cachyos,,performance\n\
fps,frametime,cpu_load,cpu_power,gpu_load,cpu_temp,gpu_temp,gpu_core_clock,gpu_mem_clock,gpu_vram_used,gpu_power,ram_used,swap_used,process_rss,cpu_mhz,elapsed\n\
49.9594,20.0163,3.33333,0,0,44,0,950,0,0,0,10.039,2.04078,0,800,39991331\n\
49.9079,20.0369,3.33333,0,0,44,0,950,0,0,0,10.039,2.04078,0,800,60029893\n";

#[test]
fn detects_mangohud_frame_header_after_metadata_rows() {
    let layout =
        detect_layout_from_reader(std::io::BufReader::new(MANGOHUD_WITH_METADATA.as_bytes()))
            .unwrap()
            .unwrap();

    assert_eq!(layout.schema.frametime_idx, 1);
    assert_eq!(layout.schema.elapsed_idx, Some(15));
    assert_eq!(layout.schema.elapsed_unit, ElapsedUnit::Nanoseconds);
    assert_eq!(
        layout.data_start_offset,
        MANGOHUD_WITH_METADATA
            .lines()
            .take(3)
            .map(|line| line.len() + 1)
            .sum::<usize>() as u64
    );
}

#[test]
fn live_parser_uses_detected_schema_for_first_tailed_data_row() {
    let layout =
        detect_layout_from_reader(std::io::BufReader::new(MANGOHUD_WITH_METADATA.as_bytes()))
            .unwrap()
            .unwrap();
    let mut parser = MangoHudLiveParser::new(layout.schema);

    let first = parser
        .parse_line("49.9594,20.0163,3.33333,0,0,44,0,950,0,0,0,10.039,2.04078,0,800,39991331")
        .unwrap();
    let second = parser
        .parse_line("49.9079,20.0369,3.33333,0,0,44,0,950,0,0,0,10.039,2.04078,0,800,60029893")
        .unwrap();

    assert_eq!(first.elapsed_ms, 39);
    assert_eq!(first.frametime_ms, 20.0163);
    assert_eq!(second.elapsed_ms, 60);
    assert_eq!(second.frametime_ms, 20.0369);
}

#[test]
fn read_frame_events_parses_mangohud_metadata_and_elapsed_nanoseconds() -> anyhow::Result<()> {
    use std::io::Write;

    let temp_dir = std::env::temp_dir().join(format!(
        "stutter_test_mangohud_metadata_{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir)?;
    let path = temp_dir.join("mangohud.csv");
    fs::File::create(&path)?.write_all(MANGOHUD_WITH_METADATA.as_bytes())?;

    let events = read_frame_events(&path, 0, None, None, None)?;

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].elapsed_ms, 0);
    assert_eq!(events[1].elapsed_ms, 21);
    assert_eq!(events[0].frametime_ms, 20.0163);
    assert_eq!(events[1].frametime_ms, 20.0369);

    fs::remove_dir_all(temp_dir).ok();
    Ok(())
}

#[test]
fn read_frame_events_respects_offset_after_mangohud_metadata() -> anyhow::Result<()> {
    use std::io::Write;

    let temp_dir = std::env::temp_dir().join(format!(
        "stutter_test_mangohud_offset_{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir)?;
    let path = temp_dir.join("mangohud.csv");
    fs::File::create(&path)?.write_all(MANGOHUD_WITH_METADATA.as_bytes())?;
    let layout = detect_layout(&path)?;

    let events = read_frame_events(&path, layout.data_start_offset, None, None, None)?;
    assert_eq!(events.len(), 2);

    let row1_len = MANGOHUD_WITH_METADATA
        .lines()
        .nth(3)
        .expect("sample has first frame row")
        .len()
        + 1;
    let events = read_frame_events(
        &path,
        layout.data_start_offset + row1_len as u64,
        None,
        None,
        None,
    )?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].frametime_ms, 20.0369);

    fs::remove_dir_all(temp_dir).ok();
    Ok(())
}

#[test]
fn skips_impossible_mangohud_reciprocal_frametimes_in_non_live_parse() {
    let header = "fps,frametime,elapsed";
    let data = "\
50.0,20.0,1000000000\n\
6.9912e-05,1.43037e+07,1045000000\n\
3.70289,270.059,1315000000\n\
50.0,20.0,1335000000\n";

    let events = parse_frame_events(
        header,
        data.lines().map(|s| Ok(s.to_owned())),
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(events.len(), 3);
    assert_eq!(events[0].elapsed_ms, 0);
    assert_eq!(events[0].frametime_ms, 20.0);
    assert_eq!(events[1].elapsed_ms, 315);
    assert_eq!(events[1].frametime_ms, 270.059);
    assert_eq!(events[2].elapsed_ms, 335);
    assert_eq!(events[2].frametime_ms, 20.0);
}

#[test]
fn live_parser_skips_impossible_mangohud_reciprocal_frametimes() {
    let headers = split_csv_line("fps,frametime,elapsed");
    let schema = schema_from_headers(&headers).unwrap();
    let mut parser = MangoHudLiveParser::new(schema);

    let first = parser.parse_line("50.0,20.0,1000000000").unwrap();
    let bad = parser.parse_line("6.9912e-05,1.43037e+07,1045000000");
    let long_real_frame = parser.parse_line("3.70289,270.059,1315000000").unwrap();
    let next = parser.parse_line("50.0,20.0,1335000000").unwrap();

    assert_eq!(first.elapsed_ms, 1000);
    assert_eq!(first.frametime_ms, 20.0);
    assert!(bad.is_none());
    assert_eq!(long_real_frame.elapsed_ms, 1315);
    assert_eq!(long_real_frame.frametime_ms, 270.059);
    assert_eq!(next.elapsed_ms, 1335);
    assert_eq!(next.frametime_ms, 20.0);
}

#[test]
fn rejects_non_positive_frametimes() {
    let header = "elapsed_ms,frametime_ms";
    let data = "10,0\n20,-1\n30,16.7\n";
    let events = parse_frame_events(
        header,
        data.lines().map(|s| Ok(s.to_owned())),
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].elapsed_ms, 0);
    assert_eq!(events[0].frametime_ms, 16.7);
}

#[test]
fn parser_fuzz_cases_reject_or_degrade_predictably() {
    enum Expected {
        Error(&'static str),
        Frames(Vec<(u64, f64)>),
    }

    let cases = [
        (
            "missing frametime header",
            "elapsed_ms,fps",
            "10,60\n",
            Expected::Error("recognized frametime column"),
        ),
        (
            "duplicate frametime header uses first recognized column",
            "elapsed_ms,frametime_ms,frametime_ms",
            "10,16.7,99.9\n",
            Expected::Frames(vec![(0, 16.7)]),
        ),
        (
            "extra columns are ignored",
            "elapsed_ms,frametime_ms",
            "10,16.7,extra,columns\n",
            Expected::Frames(vec![(0, 16.7)]),
        ),
        (
            "quoted commas stay inside fields",
            "elapsed_ms,label,frametime_ms",
            "10,\"game, menu\",16.7\n",
            Expected::Frames(vec![(0, 16.7)]),
        ),
        (
            "invalid frametime rows are skipped",
            "elapsed_ms,frametime_ms",
            "10,not-a-number\n20,16.7\n",
            Expected::Frames(vec![(0, 16.7)]),
        ),
        (
            "nanosecond elapsed units are normalized",
            "elapsed_ns,frametime_ms",
            "10000000,16.7\n26600000,16.7\n",
            Expected::Frames(vec![(0, 16.7), (16, 16.7)]),
        ),
    ];

    for (name, header, data, expected) in cases {
        let result = parse_frame_events(
            header,
            data.lines().map(|s| Ok(s.to_owned())),
            None,
            None,
            None,
        );

        match expected {
            Expected::Error(message) => {
                let err = result.expect_err(name);
                assert!(
                    err.to_string().contains(message),
                    "{name}: expected error containing {message:?}, got {err:#}"
                );
            }
            Expected::Frames(expected_frames) => {
                let events = result.unwrap_or_else(|err| panic!("{name}: {err:#}"));
                let actual = events
                    .iter()
                    .map(|event| (event.elapsed_ms, event.frametime_ms))
                    .collect::<Vec<_>>();
                assert_eq!(actual, expected_frames, "{name}");
            }
        }
    }
}

#[tokio::test]
async fn poll_alignment_skips_impossible_first_mangohud_frame() -> anyhow::Result<()> {
    use std::io::Write;

    let temp_dir = std::env::temp_dir().join(format!(
        "stutter_test_mangohud_bad_alignment_{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir)?;
    let path = temp_dir.join("mangohud.csv");
    let csv = "\
fps,frametime,elapsed\n\
6.9912e-05,1.43037e+07,1045000000\n\
50.0,20.0,1065000000\n";
    fs::File::create(&path)?.write_all(csv.as_bytes())?;
    let layout = detect_layout(&path)?;

    let (raw_ms, observed_ns) =
        poll_alignment(&path, layout.data_start_offset, Duration::from_millis(1)).await?;

    assert_eq!(raw_ms, 1065);
    assert!(observed_ns > 0);

    fs::remove_dir_all(temp_dir).ok();
    Ok(())
}

#[tokio::test]
async fn poll_alignment_uses_mangohud_elapsed_nanoseconds() -> anyhow::Result<()> {
    use std::io::Write;

    let temp_dir = std::env::temp_dir().join(format!(
        "stutter_test_mangohud_alignment_{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir)?;
    let path = temp_dir.join("mangohud.csv");
    fs::File::create(&path)?.write_all(MANGOHUD_WITH_METADATA.as_bytes())?;
    let layout = detect_layout(&path)?;

    let (raw_ms, observed_ns) =
        poll_alignment(&path, layout.data_start_offset, Duration::from_millis(1)).await?;

    assert_eq!(raw_ms, 39);
    assert!(observed_ns > 0);

    fs::remove_dir_all(temp_dir).ok();
    Ok(())
}

#[test]
fn parses_header_based_frametime_csv() {
    let header = "elapsed_ms,frametime_ms";
    let data = "10,16.7\n20,33.4\n";
    let events = parse_frame_events(
        header,
        data.lines().map(|s| Ok(s.to_owned())),
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].elapsed_ms, 0); // Normalized
    assert_eq!(events[1].elapsed_ms, 10); // 20 - 10
    assert_eq!(events[1].frametime_ms, 33.4);
}

#[test]
fn parses_quoted_csv_fields() {
    let header = "elapsed_ms,\"frame,time\",frametime_ms";
    let data = "10,\"ignored, value\",16.7\n";
    let events = parse_frame_events(
        header,
        data.lines().map(|s| Ok(s.to_owned())),
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].elapsed_ms, 0); // Normalized
    assert_eq!(events[0].frametime_ms, 16.7);
}

#[test]
fn skips_non_finite_frametimes() {
    let header = "elapsed_ms,frametime_ms";
    let data = "10,NaN\n20,inf\n30,16.7\n";
    let events = parse_frame_events(
        header,
        data.lines().map(|s| Ok(s.to_owned())),
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].elapsed_ms, 0); // Normalized (30 - 30)
    assert_eq!(events[0].frametime_ms, 16.7);
}

#[test]
fn read_frame_events_respects_newline_boundary_offset() -> anyhow::Result<()> {
    use std::io::Write;
    let temp_dir =
        std::env::temp_dir().join(format!("stutter_test_mangohud_{}", std::process::id()));
    fs::create_dir_all(&temp_dir)?;
    let path = temp_dir.join("test.csv");

    let header = "elapsed_ms,frametime_ms\n";
    let row1 = "10,16.7\n";
    let row2 = "20,33.4\n";

    let mut f = fs::File::create(&path)?;
    f.write_all(header.as_bytes())?;
    let offset_after_header = header.len() as u64;
    f.write_all(row1.as_bytes())?;
    let offset_after_row1 = offset_after_header + row1.len() as u64;
    f.write_all(row2.as_bytes())?;
    drop(f);

    // Case 1: ignore_offset = 0. Should skip header, read row1 and row2.
    let events = read_frame_events(&path, 0, None, None, None)?;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].frametime_ms, 16.7);

    // Case 2: ignore_offset = offset_after_header.
    // offset_after_header-1 is '\n'.
    // Should NOT skip the first line (row1).
    let events = read_frame_events(&path, offset_after_header, None, None, None)?;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].frametime_ms, 16.7);

    // Case 3: ignore_offset = offset_after_header + 2 (mid row1).
    // Should skip partial row1, read row2.
    let events = read_frame_events(&path, offset_after_header + 2, None, None, None)?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].frametime_ms, 33.4);

    // Case 4: ignore_offset = offset_after_row1.
    // offset_after_row1-1 is '\n'.
    // Should NOT skip row2.
    let events = read_frame_events(&path, offset_after_row1, None, None, None)?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].frametime_ms, 33.4);

    fs::remove_dir_all(temp_dir).ok();
    Ok(())
}

#[test]
fn test_alignment_with_monotonic_observed() {
    let header = "elapsed_ms,frametime_ms";
    let data = "1000,16.7\n1016,16.7\n1033,16.7\n";

    let alignment_monotonic_ns = Some(1_420_000_000); // 1420ms
    let alignment_raw_elapsed_ms = Some(1000);
    let recorder_start_monotonic_ns = Some(1_000_000_000); // 1000ms
    // observed_ms = (1420 - 1000) = 420ms

    let events = parse_frame_events(
        header,
        data.lines().map(|s| Ok(s.to_owned())),
        alignment_monotonic_ns,
        alignment_raw_elapsed_ms,
        recorder_start_monotonic_ns,
    )
    .unwrap();

    assert_eq!(events.len(), 3);
    assert_eq!(events[0].elapsed_ms, 420);
    assert_eq!(events[1].elapsed_ms, 436);
    assert_eq!(events[2].elapsed_ms, 453);
}

#[test]
fn test_alignment_missing_elapsed_column() {
    let header = "frametime_ms";
    let data = "16.7\n16.7\n16.7\n";

    let alignment_monotonic_ns = Some(1_420_000_000); // 1420ms
    let recorder_start_monotonic_ns = Some(1_000_000_000); // 1000ms
    // observed_ms = 420ms

    let events = parse_frame_events(
        header,
        data.lines().map(|s| Ok(s.to_owned())),
        alignment_monotonic_ns,
        None,
        recorder_start_monotonic_ns,
    )
    .unwrap();

    assert_eq!(events.len(), 3);
    assert_eq!(events[0].elapsed_ms, 420);
    assert_eq!(events[1].elapsed_ms, 436); // 420 + 16.7
    assert_eq!(events[2].elapsed_ms, 453); // 436 + 16.7
}

#[test]
#[ignore = "benchmark"]
fn bench_mangohud_10k_rows() {
    use std::time::Instant;
    let mut data = String::with_capacity(10_000 * 30);
    data.push_str("elapsed_ms,frametime_ms\n");
    for i in 0..10_000 {
        data.push_str(&format!("{},16.7\n", i * 16));
    }

    let start = Instant::now();
    let events = parse_frame_events(
        "elapsed_ms,frametime_ms",
        data.lines().map(|s| Ok(s.to_owned())),
        None,
        None,
        None,
    )
    .unwrap();
    let duration = start.elapsed();

    assert_eq!(events.len(), 10_000);
    println!("Parsed 10k MangoHud rows in {:?}", duration);
}
