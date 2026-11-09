use super::*;

const OUTPUT: &str = "Running 10s test @ http://127.0.0.1:8080/\n\
  Thread Distribution\n\
   50.000%    1.20ms\n\
   90.000%    2.50ms\n\
   99.000%    8.00ms\n\
   99.900%   12.00ms\n\
  1000 requests in 10.00s, 1.00MB read\n\
  Socket errors: connect 0, read 2, write 1, timeout 4\n\
  Non-2xx or 3xx responses: 3\n\
Requests/sec:    100.00\n";

#[test]
fn a_plan_is_bounded_loopback_only_and_rate_controlled() {
    let plan = plan("127.0.0.1:8080", 10, 500, 8).unwrap();
    assert_eq!(plan.program, "wrk2");
    assert_eq!(plan.offered_per_second, 500.0);
    assert!(plan.args.contains(&"-R500".to_owned()));
    assert!(plan.args.contains(&"http://127.0.0.1:8080/".to_owned()));
    assert!(
        self::plan("http://localhost:3000/health", 5, 10, 1)
            .unwrap()
            .args
            .contains(&"http://localhost:3000/health".to_owned())
    );
    assert!(self::plan("[::1]:80", 5, 10, 1).is_ok());
}

#[test]
fn remote_hosts_https_and_out_of_range_arguments_are_refused() {
    for target in [
        "example.com:80",
        "10.0.0.5:80",
        "192.168.1.2:8080",
        "0.0.0.0:80",
        "https://127.0.0.1:443",
        "127.0.0.1",
        "127.0.0.1:0",
        "127.0.0.1:notaport",
    ] {
        assert!(plan(target, 5, 10, 1).is_err(), "{target}");
    }
    for (seconds, rate, connections) in [
        (0, 10, 1),
        (MAX_SECONDS + 1, 10, 1),
        (5, 0, 1),
        (5, MAX_RATE + 1, 1),
        (5, 10, 0),
        (5, 10, MAX_CONNECTIONS + 1),
    ] {
        assert!(
            plan("127.0.0.1:80", seconds, rate, connections).is_err(),
            "{seconds} {rate} {connections}"
        );
    }
}

#[test]
fn wrk2_output_becomes_a_load_run_with_errors_counted_and_p99_in_milliseconds() {
    let run = import_wrk2(OUTPUT, 100.0).unwrap();
    assert_eq!(run.completed, 1000);
    assert_eq!(run.errors, 2 + 1 + 4 + 3);
    assert_eq!(run.p99_ms, 8.0);
    assert_eq!(run.offered_per_second, 100.0);
    assert_eq!(millis("850.00us"), Some(0.85));
    assert_eq!(millis("1.50s"), Some(1500.0));
}

#[test]
fn output_without_a_result_is_an_error_not_a_zero_run() {
    assert!(import_wrk2("", 1.0).is_err());
    assert!(import_wrk2("0 requests in 10.00s\n   99.000%    1.00ms\n", 1.0).is_err());
    assert!(import_wrk2("100 requests in 10.00s\n", 1.0).is_err());
}
