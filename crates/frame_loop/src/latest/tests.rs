use super::*;
use std::{
    cell::Cell,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::Duration,
};

#[test]
fn latest_replaces_only_the_unconsumed_value() {
    let mut slot = latest();
    let (mut publisher, mut consumer) = slot.split();
    publisher.publish(vec![1, 2]).unwrap();
    let owned = consumer.take().unwrap();
    publisher.publish(vec![3, 4]).unwrap();
    publisher.publish(vec![5, 6]).unwrap();
    assert_eq!(owned, [1, 2]);
    assert_eq!(consumer.take(), Some(vec![5, 6]));
    assert_eq!(consumer.take(), None);
}

#[test]
fn closing_publisher_keeps_the_last_value() {
    let mut slot = latest();
    let (mut publisher, mut consumer) = slot.split();
    publisher.publish(17).unwrap();
    drop(publisher);
    assert_eq!(consumer.wait_until(Instant::now()), Wait::Value(17));
    assert_eq!(consumer.wait_until(Instant::now()), Wait::Closed);
}

#[test]
fn closing_consumer_returns_ownership_to_publisher() {
    let mut slot = latest();
    let (mut publisher, consumer) = slot.split();
    drop(consumer);
    assert_eq!(publisher.publish(vec![3, 5]).unwrap_err().0, [3, 5]);
}

#[test]
fn expired_wait_checks_the_slot_before_timing_out() {
    let mut slot = latest();
    let (mut publisher, mut consumer) = slot.split();
    let expired = Instant::now();
    assert_eq!(consumer.wait_until(expired), Wait::Timeout);
    publisher.publish(9).unwrap();
    assert_eq!(consumer.wait_until(expired), Wait::Value(9));
}

#[test]
fn cross_thread_payload_requires_send_but_not_sync() {
    let mut slot = latest::<Cell<u32>>();
    thread::scope(|scope| {
        let (mut publisher, mut consumer) = slot.split();
        let worker = scope.spawn(move || {
            match consumer.wait_until(Instant::now() + Duration::from_secs(5)) {
                Wait::Value(value) => value.get(),
                result => panic!("未取得值：{result:?}"),
            }
        });
        publisher.publish(Cell::new(23)).unwrap();
        assert_eq!(worker.join().unwrap(), 23);
    });
}

#[test]
fn concurrent_handoff_keeps_complete_packets_in_order() {
    let mut slot = latest::<[u64; 64]>();
    thread::scope(|scope| {
        let (mut publisher, mut consumer) = slot.split();
        let worker = scope.spawn(move || {
            let mut last = 0;
            loop {
                match consumer.wait_until(Instant::now() + Duration::from_secs(5)) {
                    Wait::Value(packet) => {
                        assert!(packet[0] > last);
                        assert!(packet.iter().all(|&value| value == packet[0]));
                        last = packet[0];
                    }
                    Wait::Closed => return last,
                    Wait::Timeout => panic!("生产端没有关闭"),
                }
            }
        });
        for sequence in 1..=4_096 {
            publisher.publish([sequence; 64]).unwrap();
        }
        drop(publisher);
        assert_eq!(worker.join().unwrap(), 4_096);
    });
}

#[test]
fn closing_publisher_releases_a_waiting_consumer() {
    let mut slot = latest::<u32>();
    thread::scope(|scope| {
        let (publisher, mut consumer) = slot.split();
        let worker =
            scope.spawn(move || consumer.wait_until(Instant::now() + Duration::from_secs(5)));
        drop(publisher);
        assert_eq!(worker.join().unwrap(), Wait::Closed);
    });
}

struct DropProbe<'a> {
    started: std::sync::mpsc::Sender<()>,
    checked: &'a Mutex<std::sync::mpsc::Receiver<bool>>,
    drops: &'a AtomicUsize,
}

impl Drop for DropProbe<'_> {
    fn drop(&mut self) {
        self.started.send(()).unwrap();
        assert!(self.checked.lock().unwrap().recv().unwrap());
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn discarded_values_are_destroyed_once_outside_the_lock() {
    let drops = AtomicUsize::new(0);
    let (started, requests) = std::sync::mpsc::channel();
    let (checked, results) = std::sync::mpsc::channel();
    let results = Mutex::new(results);
    let mut slot = latest();
    let (mut publisher, consumer) = slot.split();
    let shared = publisher.shared;
    thread::scope(|scope| {
        let worker = scope.spawn(move || {
            for _ in 0..2 {
                requests.recv().unwrap();
                checked.send(shared.state.try_lock().is_ok()).unwrap();
            }
        });
        let probe = || DropProbe {
            started: started.clone(),
            checked: &results,
            drops: &drops,
        };
        publisher.publish(probe()).unwrap();
        publisher.publish(probe()).unwrap();
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        drop(consumer);
        assert_eq!(drops.load(Ordering::Relaxed), 2);
        worker.join().unwrap();
    });
}

#[test]
fn poisoned_lock_preserves_the_slot_contract() {
    let mut slot = latest::<u32>();
    let (mut publisher, mut consumer) = slot.split();
    let shared = publisher.shared;
    thread::scope(|scope| {
        assert!(
            scope
                .spawn(move || {
                    let _guard = shared.state.lock().unwrap();
                    panic!("模拟锁持有者异常退出");
                })
                .join()
                .is_err()
        );
    });
    publisher.publish(29).unwrap();
    assert_eq!(consumer.take(), Some(29));
}
