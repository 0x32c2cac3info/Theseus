/// Handles the Programmable Interval Timer (PIT) system clock,
/// which allows us to configure the frequency of timer interrupts. 

use cpuio::Port;
use spin::Mutex;
use core::sync::atomic::{AtomicUsize, Ordering};

/// the main interrupt channel
const CHANNEL0: u16 = 0x40;
/// DO NOT USE
const CHANNEL1: u16 = 0x41;
/// the speaker for beeping
const CHANNEL2: u16 = 0x42;

const COMMAND_REGISTER: u16 = 0x43;


/// the timer's default frequency is 1.19 MHz
const PIT_DIVIDEND_HZ: u32 = 1193182; 

static PIT_COMMAND: Mutex<Port<u8>> = Mutex::new( unsafe{ Port::new(COMMAND_REGISTER) } );
static PIT_CHANNEL_0: Mutex<Port<u8>> = Mutex::new( unsafe{ Port::new(CHANNEL0) } );


// static TICKS: AtomicUsize = AtomicUsize::new(0); // default()??
pub static mut TICKS: u64 = 0;


pub fn init(freq_hertz: u32) {
    let divisor = PIT_DIVIDEND_HZ / freq_hertz;
    PIT_COMMAND.lock().write(0x36); // 0x36: see this: http://www.osdever.net/bkerndev/Docs/pit.htm
    // PIT_COMMAND.lock().write(0x34); // some other mode that we don't want

    if (divisor > u16::max_value() as u32) {
        panic!("The chosen PIT frequency ({} Hz) is too small, it must be higher than 18 Hz!", freq_hertz);
    }

    // must write the low byte and then the high byte
    PIT_CHANNEL_0.lock().write(divisor as u8);
    PIT_CHANNEL_0.lock().write((divisor >> 8) as u8);
}


// these should be moved to a config file

/// the chosen interrupt frequency (in Hertz) of the PIT clock 
const PIT_FREQUENCY_HZ: u64 = 100; 

/// the timeslice period in milliseconds
const timeslice_period_ms: u64 = 100; 

/// the heartbeatperiod in milliseconds
const heartbeat_period_ms: u64 = 10000;

/// this occurs on every PIT timer tick, which is currently 100 Hz
pub fn handle_timer_interrupt() {
    let ticks = unsafe {
        TICKS += 1;
        TICKS
    };


    // preemption timeslice = 1sec (every 100 ticks)
    if (ticks % (timeslice_period_ms * PIT_FREQUENCY_HZ / 1000)) == 0 {
        // FIXME: if we call schedule() too frequently, like on every tick,  the system locks up!
        // Most likely because we acquire locks in the scheduler/context switching routines
        schedule!();
    }

    // heartbeat: print every 10 seconds
    if (ticks % (heartbeat_period_ms * PIT_FREQUENCY_HZ / 1000)) == 0 {
        trace!("[heartbeat] {} seconds have passed (ticks={})", heartbeat_period_ms/1000, ticks);
        // info!("1 second has passed (ticks={})", ticks);
    }
}