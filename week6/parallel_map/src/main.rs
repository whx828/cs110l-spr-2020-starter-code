use crossbeam_channel;
use std::{thread, time};

fn parallel_map<T, U, F>(mut input_vec: Vec<T>, num_threads: usize, f: F) -> Vec<U>
where
    F: FnOnce(T) -> U + Send + Copy + 'static,
    T: Send + 'static,
    U: Send + 'static + Default,
{
    let mut output_vec: Vec<U> = Vec::with_capacity(input_vec.len());

    for _ in 0..input_vec.len() {
        output_vec.push(U::default());
    }

    // TODO: implement parallel map!
    let (sender1, receiver1) = crossbeam_channel::unbounded();
    let (sender2, receiver2) = crossbeam_channel::unbounded();
    let mut threads = Vec::new();

    for _ in 0..num_threads {
        let receiver = receiver1.clone();
        let sender = sender2.clone();

        threads.push(thread::spawn(move || {
            while let Ok((i, t)) = receiver.recv() {
                sender.send((i, f(t))).expect("msg");
            }
        }));
    }

    drop(sender2);

    for (i, t) in input_vec.into_iter().enumerate() {
        sender1.send((i, t)).expect("msg");
    }

    drop(sender1);

    while let Ok((i, u)) = receiver2.recv() {
        output_vec[i] = u;
    }

    output_vec
}

fn main() {
    let v = vec![6, 7, 8, 9, 10, 1, 2, 3, 4, 5, 12, 18, 11, 5, 20];
    let squares = parallel_map(v, 10, |num| {
        println!("{} squared is {}", num, num * num);
        thread::sleep(time::Duration::from_millis(500));
        num * num
    });
    println!("squares: {:?}", squares);
}
