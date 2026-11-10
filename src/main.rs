use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};


mod savelog;
mod sense;



let running = Arc::new(AtomicBool::new(true));
//not cloning, since i used ARC, this is a second pointer to the same heap space
let r = running.clone(); 

ctrlc::set_handler(move || {
    r.store(false, Ordering::SeqCst);
})?;


fn main() {
    // read command args

    while running.load(Ordering::SeqCst) {
        // read sensor data
        // execute sense.rs
        // save data in temporal log
        // either here or in sense.rs

        // nap for configured time
    }
}
