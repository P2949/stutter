use super::*;

#[test]
fn test_build_wake_graph_grouping_and_sorting() {
    let points = vec![
        SpikePoint {
            task: 101,
            comm: "wakee1".to_owned(),
            waker_tid: 201,
            waker_comm: "waker1".to_owned(),
            latency_ns: 1000,
            ..SpikePoint::default()
        },
        SpikePoint {
            task: 101,
            comm: "wakee1".to_owned(),
            waker_tid: 201,
            waker_comm: "waker1".to_owned(),
            latency_ns: 2000,
            ..SpikePoint::default()
        },
        SpikePoint {
            task: 102,
            comm: "wakee2".to_owned(),
            waker_tid: 201,
            waker_comm: "waker1".to_owned(),
            latency_ns: 500,
            ..SpikePoint::default()
        },
        SpikePoint {
            task: 101,
            comm: "wakee1".to_owned(),
            waker_tid: 202,
            waker_comm: "waker2".to_owned(),
            latency_ns: 5000,
            ..SpikePoint::default()
        },
    ];

    let graph = build_wake_graph(&points);

    // Should have 3 edges:
    // 1. (201, waker1) -> (101, wakee1) count=2 max_lat=2000
    // 2. (202, waker2) -> (101, wakee1) count=1 max_lat=5000
    // 3. (201, waker1) -> (102, wakee2) count=1 max_lat=500

    // Sorted by count desc, then max_lat desc
    assert_eq!(graph.len(), 3);

    assert_eq!(graph[0].waker_tid, 201);
    assert_eq!(graph[0].wakee_tid, 101);
    assert_eq!(graph[0].count, 2);
    assert_eq!(graph[0].max_latency_ns, 2000);

    assert_eq!(graph[1].waker_tid, 202);
    assert_eq!(graph[1].count, 1);
    assert_eq!(graph[1].max_latency_ns, 5000);

    assert_eq!(graph[2].waker_tid, 201);
    assert_eq!(graph[2].wakee_tid, 102);
    assert_eq!(graph[2].count, 1);
    assert_eq!(graph[2].max_latency_ns, 500);
}
