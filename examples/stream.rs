extern crate tokio_core;
extern crate futures;
extern crate tokio_token_bucket;
extern crate rand;

use std::time::{Duration};
use tokio_token_bucket::{Config, TokenBucket};
use futures::{Stream};
use tokio_core::reactor::{Core};
use futures::stream;
use futures::{Future};
use rand::Rng;

fn main() {
    let mut rng = rand::thread_rng();

    let mut core = Core::new().unwrap();

    //    let mut mb = MultiBar::new();
    //    let count = 100;
    //    mb.println("Rate");
    //    let mut p1 = mb.create_bar(count);
    //    p1.finish();
    //    mb.println("Params");
    //    let mut interval_ms = mb.create_bar(count * 2);
    //    interval_ms.finish();
    //    mb.listen();

    let config = Config {
        capacity: 2000.0,
        quantum: 20.0,
        interval: Duration::new(0, 20000000),
    };

    let handle = core.handle();
    let token_bucket = TokenBucket::configured(config, &handle);

    let token_bucket_handle = token_bucket.handle();

    let stream = stream::repeat::<_, bool>(1).map(|_| {
        let size = rng.gen::<u8>() as usize;
        let token_bucket_handle = token_bucket_handle.clone();
        match rng.gen::<u8>() % 10 {
            1 => {
                let quantum = rng.gen::<u8>() as f64;
                println!("Set quantum to {}", quantum);
                token_bucket_handle.set_quantum(quantum);
            }
            2 => {
                let ms = (rng.gen::<u16>() % 1000) as u64;
                println!("Set interval to {}ms", ms);
                token_bucket_handle.set_interval(Duration::from_millis(ms));
            }
            3 => {
                let capacity = (rng.gen::<u64>() % 10000) as f64;
                println!("Set capacity to {}", capacity);
                token_bucket_handle.set_capacity(capacity);
            },
            _ => {},
        }
        ((), size)
    });

    let (limiter_sink, limiter_stream) = token_bucket.limiter().split();

    let reader = limiter_stream;

    handle.spawn(token_bucket.then(|_| Ok(())));
    handle.spawn(reader.for_each(|_| Ok(())).then(|_| Ok(())));

    let fut = stream.forward(limiter_sink);
    core.run(fut).unwrap();
}
