// Copyright 2016 Philipp Oppermann. See the README.md
// file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use x86_64::structures::tss::TaskStateSegment;
use x86_64::structures::idt::{Idt, ExceptionStackFrame, PageFaultErrorCode};
use spin::{Mutex, Once};
use port_io::Port;
use drivers::input::keyboard;
use drivers::ata_pio;
use arch;
use CONFIG::*;
use x86_64::structures::gdt::SegmentSelector;


// expose these functions from within this interrupt module
pub use irq_safety::{disable_interrupts, enable_interrupts, interrupts_enabled};


mod gdt;
pub mod pit_clock; // TODO: shouldn't be pub
mod pic;
mod time_tools; //testing whether including a module makes any difference
pub mod rtc; // TODO: shouldn't be pub
pub mod tsc;



const DOUBLE_FAULT_IST_INDEX: usize = 0;


static KERNEL_CODE_SELECTOR: Once<SegmentSelector> = Once::new();
static KERNEL_DATA_SELECTOR: Once<SegmentSelector> = Once::new();
static USER_CODE_32_SELECTOR: Once<SegmentSelector> = Once::new();
static USER_DATA_32_SELECTOR: Once<SegmentSelector> = Once::new();
static USER_CODE_64_SELECTOR: Once<SegmentSelector> = Once::new();
static USER_DATA_64_SELECTOR: Once<SegmentSelector> = Once::new();
static TSS_SELECTOR: Once<SegmentSelector> = Once::new();


lazy_static! {
    static ref IDT: Idt = {
        let mut idt = ::x86_64::structures::idt::Idt::new();

		// SET UP FIXED EXCEPTION HANDLERS
        idt.divide_by_zero.set_handler_fn(divide_by_zero_handler);
        // missing: 0x01 debug exception
        // missing: 0x02 non-maskable interrupt exception
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        // missing: 0x04 overflow exception
        // missing: 0x05 bound range exceeded exception
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        idt.device_not_available.set_handler_fn(device_not_available_handler);
        unsafe {
            idt.double_fault.set_handler_fn(double_fault_handler)
                .set_stack_index(DOUBLE_FAULT_IST_INDEX as u16); // use a special stack for the DF handler
        }
        // reserved: 0x09 coprocessor segment overrun exception
        // missing: 0x0a invalid TSS exception
        idt.segment_not_present.set_handler_fn(segment_not_present_handler);
        // missing: 0x0c stack segment exception
        idt.general_protection_fault.set_handler_fn(general_protection_fault_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        // reserved: 0x0f vector 15
        // missing: 0x10 floating point exception
        // missing: 0x11 alignment check exception
        // missing: 0x12 machine check exception
        // missing: 0x13 SIMD floating point exception
        // missing: 0x14 virtualization vector 20
        // missing: 0x15 - 0x1d SIMD floating point exception
        // missing: 0x1e security exception
        // reserved: 0x1f


        // fill all IDT entries with an unimplemented IRQ handler
        for i in 32..255 {
	        idt[i].set_handler_fn(unimplemented_interrupt_handler);
        }


		// SET UP CUSTOM INTERRUPT HANDLERS
		// we can directly index the "idt" object because it implements the Index/IndexMut traits
        idt[0x20].set_handler_fn(timer_handler); // int 32
        idt[0x21].set_handler_fn(keyboard_handler); // int 33
        idt[0x27].set_handler_fn(spurious_interrupt_handler); 

        //if interrupt is correct, will send to rtc_handler function rtc-test
        idt[0x28].set_handler_fn(rtc_handler);
        idt[0x2e].set_handler_fn(primary_ata);


        // TODO: add more 


        idt // return idt so it's set to the static ref IDT above
    };
}

pub enum AvailableSegmentSelector {
    KernelCode,
    KernelData,
    UserCode32,
    UserData32,
    UserCode64,
    UserData64,
    Tss,
}


/// Stupid hack because SegmentSelector is not Cloneable/Copyable
pub fn get_segment_selector(selector: AvailableSegmentSelector) -> SegmentSelector {
    let seg: &SegmentSelector = match selector {
        AvailableSegmentSelector::KernelCode => {
            KERNEL_CODE_SELECTOR.try().expect("KERNEL_CODE_SELECTOR failed to init!")
        }
        AvailableSegmentSelector::KernelData => {
            KERNEL_DATA_SELECTOR.try().expect("KERNEL_DATA_SELECTOR failed to init!")
        }
        AvailableSegmentSelector::UserCode32 => {
            USER_CODE_32_SELECTOR.try().expect("USER_CODE_32_SELECTOR failed to init!")
        }
        AvailableSegmentSelector::UserData32 => {
            USER_DATA_32_SELECTOR.try().expect("USER_DATA__32SELECTOR failed to init!")
        }
        AvailableSegmentSelector::UserCode64 => {
            USER_CODE_64_SELECTOR.try().expect("USER_CODE_32_SELECTOR failed to init!")
        }
        AvailableSegmentSelector::UserData64 => {
            USER_DATA_64_SELECTOR.try().expect("USER_DATA__32SELECTOR failed to init!")
        }
        AvailableSegmentSelector::Tss => {
            TSS_SELECTOR.try().expect("TSS_SELECTOR failed to init!")
        }
    };

    SegmentSelector::new(seg.index(), seg.rpl())
}



/// Interface to our PIC (programmable interrupt controller) chips.
/// We want to map hardware interrupts to 0x20 (for PIC1) or 0x28 (for PIC2).
static mut PIC: pic::ChainedPics = unsafe { pic::ChainedPics::new(0x20, 0x28) };
static KEYBOARD: Mutex<Port<u8>> = Mutex::new(Port::new(0x60));

static TSS: Once<TaskStateSegment> = Once::new();
static GDT: Once<gdt::Gdt> = Once::new();


/// initializes the interrupt subsystem and IRQ handlers with exceptions
/// Arguments: the address of the top of a newly allocated stack, to be used as the double fault exception handler stack 
/// Arguments: the address of the top of a newly allocated stack, to be used as the privilege stack (Ring 3 -> Ring 0 stack)
pub fn init(double_fault_stack_top_unusable: usize, privilege_stack_top_unusable: usize) {
    assert_has_not_been_called!("interrupts::init was called more than once!");

    
    use x86_64::instructions::segmentation::{set_cs, load_ds, load_ss};
    use x86_64::instructions::tables::load_tss;
    use x86_64::PrivilegeLevel;
    use x86_64::VirtualAddress;

    

    let tss = TSS.call_once(|| {
                                let mut tss = TaskStateSegment::new();
                                // TSS.RSP0 is used in kernel space after a transition from Ring 3 -> Ring 0
                                tss.privilege_stack_table[0] = VirtualAddress(privilege_stack_top_unusable);
                                tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX] = VirtualAddress(double_fault_stack_top_unusable);
                                tss
                            });



    let gdt = GDT.call_once(|| {
        let mut gdt = gdt::Gdt::new();

        // this order of code segments must be preserved: kernel cs, kernel ds, user cs 32, user ds 32, user cs 64, user ds 64, tss

        KERNEL_CODE_SELECTOR.call_once(|| {
            gdt.add_entry(gdt::Descriptor::kernel_code_segment(), PrivilegeLevel::Ring0)
        });
        KERNEL_DATA_SELECTOR.call_once(|| {
            gdt.add_entry(gdt::Descriptor::kernel_data_segment(), PrivilegeLevel::Ring0)
        });
        USER_CODE_32_SELECTOR.call_once(|| {
            gdt.add_entry(gdt::Descriptor::user_code_32_segment(), PrivilegeLevel::Ring3)
        });
        USER_DATA_32_SELECTOR.call_once(|| {
            gdt.add_entry(gdt::Descriptor::user_data_32_segment(), PrivilegeLevel::Ring3)
        });
        USER_CODE_64_SELECTOR.call_once(|| {
            gdt.add_entry(gdt::Descriptor::user_code_64_segment(), PrivilegeLevel::Ring3)
        });
        USER_DATA_64_SELECTOR.call_once(|| {
            gdt.add_entry(gdt::Descriptor::user_data_64_segment(), PrivilegeLevel::Ring3)
        });
        TSS_SELECTOR.call_once(|| {
            gdt.add_entry(gdt::Descriptor::tss_segment(&tss), PrivilegeLevel::Ring0)
        });
        gdt
    });
    gdt.load();

    println_unsafe!("Loaded GDT: {}", gdt);

    unsafe {
        set_cs(get_segment_selector(AvailableSegmentSelector::KernelCode)); // reload code segment register
        load_tss(get_segment_selector(AvailableSegmentSelector::Tss)); // load TSS
        
        load_ss(get_segment_selector(AvailableSegmentSelector::KernelData)); // unsure if necessary
        load_ds(get_segment_selector(AvailableSegmentSelector::KernelData)); // unsure if necessary

        PIC.initialize();
    }

    IDT.load();
    println_unsafe!("loaded interrupt descriptor table.");

    // init PIT and RTC interrupts
    pit_clock::init(CONFIG_PIT_FREQUENCY_HZ);
    rtc::enable_rtc_interrupt();
    rtc::change_rtc_frequency(CONFIG_RTC_FREQUENCY_HZ);
}



/// interrupt 0x00
extern "x86-interrupt" fn divide_by_zero_handler(stack_frame: &mut ExceptionStackFrame) {
    println_unsafe!("\nEXCEPTION: DIVIDE BY ZERO\n{:#?}", stack_frame);
    loop {}
}

/// interrupt 0x03
extern "x86-interrupt" fn breakpoint_handler(stack_frame: &mut ExceptionStackFrame) {
    println_unsafe!("\nEXCEPTION: BREAKPOINT at {:#x}\n{:#?}",
             stack_frame.instruction_pointer,
             stack_frame);
}

/// interrupt 0x06
extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: &mut ExceptionStackFrame) {
    println_unsafe!("\nEXCEPTION: INVALID OPCODE at {:#x}\n{:#?}",
             stack_frame.instruction_pointer,
             stack_frame);
    loop {}
}

/// interrupt 0x07
/// see this: http://wiki.osdev.org/I_Cant_Get_Interrupts_Working#I_keep_getting_an_IRQ7_for_no_apparent_reason
extern "x86-interrupt" fn device_not_available_handler(stack_frame: &mut ExceptionStackFrame) {
    println_unsafe!("\nEXCEPTION: DEVICE_NOT_AVAILABLE at {:#x}\n{:#?}",
             stack_frame.instruction_pointer,
             stack_frame);

}



extern "x86-interrupt" fn page_fault_handler(stack_frame: &mut ExceptionStackFrame, error_code: PageFaultErrorCode) {
    use x86_64::registers::control_regs;
    println_unsafe!("\nEXCEPTION: PAGE FAULT while accessing {:#x}\nerror code: \
                                  {:?}\n{:#?}",
             control_regs::cr2(),
             error_code,
             stack_frame);
    loop {}
}

extern "x86-interrupt" fn double_fault_handler(stack_frame: &mut ExceptionStackFrame, _error_code: u64) {
    println_unsafe!("\nEXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
    loop {}
}



/// this shouldn't really ever happen, but I added the handler anyway
/// because I noticed the interrupt 0xb happening when other interrupts weren't properly handled
extern "x86-interrupt" fn segment_not_present_handler(stack_frame: &mut ExceptionStackFrame, error_code: u64) {
    use x86_64::registers::control_regs;
    println_unsafe!("\nEXCEPTION: SEGMENT_NOT_PRESENT FAULT\nerror code: \
                                  {:#b}\n{:#?}",
//             control_regs::cr2(),
             error_code,
             stack_frame);

    loop {}
}


extern "x86-interrupt" fn general_protection_fault_handler(stack_frame: &mut ExceptionStackFrame, error_code: u64) {
    println_unsafe!("\nEXCEPTION: GENERAL PROTECTION FAULT \nerror code: \
                                  {:#b}\n{:#?}",
             error_code,
             stack_frame);


    // TODO: kill the offending process
    loop {}
}





// 0x20
extern "x86-interrupt" fn timer_handler(stack_frame: &mut ExceptionStackFrame) {
    // this is how to write something with literally ZERO locking
    // TODO: FIXME: establish non-locking debug messages, with compile-time string literals only!
    // we still do not know how to print runtime values without locking, due to the format!() macro needing allocation.
    // ::drivers::serial_port::serial_out("\n\x1b[33m[W] TIMER! \x1b[0m\n");

    // we must acknowledge the interrupt first before handling it, which will cause a context switch
	unsafe { PIC.notify_end_of_interrupt(0x20); }
    //time_tools::return_ticks();

    pit_clock::handle_timer_interrupt();
}


// 0x21
extern "x86-interrupt" fn keyboard_handler(stack_frame: &mut ExceptionStackFrame) {
    // in this interrupt, we must read the keyboard scancode register before acknowledging the interrupt.
    let mut scan_code: u8 = { 
        KEYBOARD.lock().read() 
    };
	// trace!("KBD: {:?}", scan_code);


    keyboard::handle_keyboard_input(scan_code);	
    unsafe { PIC.notify_end_of_interrupt(0x21); }
    
}


static MASTER_PIC_CMD_REG: Port<u8>  = Port::new(0x20);
//0x27
extern "x86-interrupt" fn spurious_interrupt_handler(stack_frame: &mut ExceptionStackFrame ) {
    // println_unsafe!("\nSPURIOUS IRQ");

    unsafe {
        MASTER_PIC_CMD_REG.write(0x0B);
        let isr = MASTER_PIC_CMD_REG.read();

        MASTER_PIC_CMD_REG.write(0x0A);
        let irr = MASTER_PIC_CMD_REG.read();


        println_unsafe!("\nSpurious interrupt handler:  isr={:#b} irr={:#b}\n", isr, irr);
        if isr & 0x80 == 0x80 {
            PIC.notify_end_of_interrupt(0x27);
        }
        else {
            // do nothing
        }
    }

	// TODO: handle this
	/* When any IRQ7 is received, simply read the In-Service Register
		 outb(0x20, 0x0B); unsigned char irr = inb(0x20);
		and check if bit 7
		irr & 0x80
		is set. If it isn't, then return from the interrupt without sending an EOI.
	*/
}



//0x28
extern "x86-interrupt" fn rtc_handler(stack_frame: &mut ExceptionStackFrame ) {
    unsafe { PIC.notify_end_of_interrupt(0x28); }

    //let placeholder = 2;
    //trace!("wow");
    rtc::handle_rtc_interrupt();

    

}

//0x2e
extern "x86-interrupt" fn primary_ata(stack_frame:&mut ExceptionStackFrame ) {
    unsafe { PIC.notify_end_of_interrupt(0x2e); }

    //let placeholder = 2;
    
    ata_pio::handle_primary_interrupt();

    

}

extern "x86-interrupt" fn unimplemented_interrupt_handler(stack_frame: &mut ExceptionStackFrame) {
	println_unsafe!("caught unhandled interrupt: {:#?}", stack_frame);

    loop { }
}