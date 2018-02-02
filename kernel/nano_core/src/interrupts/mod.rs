// Copyright 2016 Philipp Oppermann. See the README.md
// file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use x86_64;
use x86_64::structures::tss::TaskStateSegment;
use x86_64::structures::idt::{LockedIdt, ExceptionStackFrame};
use spin::{Mutex, Once};
use port_io::Port;
use drivers::input::keyboard;
use drivers::ata_pio;
use kernel_config::time::{CONFIG_PIT_FREQUENCY_HZ, CONFIG_TIMESLICE_PERIOD_MS, CONFIG_RTC_FREQUENCY_HZ};
use x86_64::structures::gdt::SegmentSelector;
use rtc;
use atomic::{Ordering, Atomic};
use atomic_linked_list::atomic_map::AtomicMap;
use memory::VirtualAddress;


mod exceptions;
mod gdt;
pub mod pit_clock; // TODO: shouldn't be pub
pub mod apic;
pub mod ioapic;
mod pic;
pub mod tsc;


// re-expose these functions from within this interrupt module
pub use irq_safety::{disable_interrupts, enable_interrupts, interrupts_enabled};
pub use self::exceptions::init_early_exceptions;

/// The index of the double fault stack in a TaskStateSegment (TSS)
const DOUBLE_FAULT_IST_INDEX: usize = 0;


static KERNEL_CODE_SELECTOR:  Once<SegmentSelector> = Once::new();
static KERNEL_DATA_SELECTOR:  Once<SegmentSelector> = Once::new();
static USER_CODE_32_SELECTOR: Once<SegmentSelector> = Once::new();
static USER_DATA_32_SELECTOR: Once<SegmentSelector> = Once::new();
static USER_CODE_64_SELECTOR: Once<SegmentSelector> = Once::new();
static USER_DATA_64_SELECTOR: Once<SegmentSelector> = Once::new();
static TSS_SELECTOR:          Once<SegmentSelector> = Once::new();


/// The single system-wide IDT
/// Note: this could be per-core instead of system-wide, if needed.
pub static IDT: LockedIdt = LockedIdt::new();

/// Interface to our PIC (programmable interrupt controller) chips.
/// We want to map hardware interrupts to 0x20 (for PIC1) or 0x28 (for PIC2).
static PIC: Once<pic::ChainedPics> = Once::new();
static KEYBOARD: Mutex<Port<u8>> = Mutex::new(Port::new(0x60));

/// The TSS list, one per core, indexed by a key of apic_id
lazy_static! {
    static ref TSS: AtomicMap<u8, TaskStateSegment> = AtomicMap::new();
}
/// The GDT list, one per core, indexed by a key of apic_id
lazy_static! {
    static ref GDT: AtomicMap<u8, gdt::Gdt> = AtomicMap::new();
}

pub static INTERRUPT_CHIP: Atomic<InterruptChip> = Atomic::new(InterruptChip::APIC);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InterruptChip {
    APIC,
    x2apic,
    PIC,
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
            KERNEL_CODE_SELECTOR.try().expect("KERNEL_CODE_SELECTOR wasn't yet inited!")
        }
        AvailableSegmentSelector::KernelData => {
            KERNEL_DATA_SELECTOR.try().expect("KERNEL_DATA_SELECTOR wasn't yet inited!")
        }
        AvailableSegmentSelector::UserCode32 => {
            USER_CODE_32_SELECTOR.try().expect("USER_CODE_32_SELECTOR wasn't yet inited!")
        }
        AvailableSegmentSelector::UserData32 => {
            USER_DATA_32_SELECTOR.try().expect("USER_DATA_32_SELECTOR wasn't yet inited!")
        }
        AvailableSegmentSelector::UserCode64 => {
            USER_CODE_64_SELECTOR.try().expect("USER_CODE_64_SELECTOR wasn't yet inited!")
        }
        AvailableSegmentSelector::UserData64 => {
            USER_DATA_64_SELECTOR.try().expect("USER_DATA_64_SELECTOR wasn't yet inited!")
        }
        AvailableSegmentSelector::Tss => {
            TSS_SELECTOR.try().expect("TSS_SELECTOR wasn't yet inited!")
        }
    };

    SegmentSelector::new(seg.index(), seg.rpl())
}




/// Sets the current core's TSS privilege stack 0 (RSP0) entry, which points to the stack that 
/// the x86_64 hardware automatically switches to when transitioning from Ring 3 -> Ring 0.
/// Should be set to an address within the current userspace task's kernel stack.
/// WARNING: If set incorrectly, the OS will crash upon an interrupt from userspace into kernel space!!
pub fn tss_set_rsp0(new_privilege_stack_top: usize) -> Result<(), &'static str> {
    let my_apic_id = try!(apic::get_my_apic_id().ok_or("couldn't get_my_apic_id"));
    let mut tss_entry = try!(TSS.get_mut(my_apic_id).ok_or_else(|| {
        error!("tss_set_rsp0(): couldn't find TSS for apic {}", my_apic_id);
        "No TSS for the current core's apid id" 
    }));
    tss_entry.privilege_stack_table[0] = x86_64::VirtualAddress(new_privilege_stack_top);
    // trace!("tss_set_rsp0: new TSS {:?}", tss_entry);
    Ok(())
}



/// initializes the interrupt subsystem and properly sets up safer exception-related IRQs, but no other IRQ handlers.
/// Arguments: the address of the top of a newly allocated stack, to be used as the double fault exception handler stack 
/// Arguments: the address of the top of a newly allocated stack, to be used as the privilege stack (Ring 3 -> Ring 0 stack)
pub fn init(double_fault_stack_top_unusable: VirtualAddress, privilege_stack_top_unusable: VirtualAddress) 
       -> Result<(), &'static str> {
    let bsp_id = try!(apic::get_bsp_id().ok_or("couldn't get BSP's id"));
    info!("Setting up TSS & GDT for BSP (id {})", bsp_id);
    create_tss_gdt(bsp_id, double_fault_stack_top_unusable, privilege_stack_top_unusable);

    {
        let mut idt = IDT.lock(); // withholds interrupts

        // SET UP FIXED EXCEPTION HANDLERS
        idt.divide_by_zero.set_handler_fn(exceptions::divide_by_zero_handler);
        // missing: 0x01 debug exception
        // missing: 0x02 non-maskable interrupt exception
        idt.breakpoint.set_handler_fn(exceptions::breakpoint_handler);
        // missing: 0x04 overflow exception
        // missing: 0x05 bound range exceeded exception
        idt.invalid_opcode.set_handler_fn(exceptions::invalid_opcode_handler);
        idt.device_not_available.set_handler_fn(exceptions::device_not_available_handler);
        unsafe {
            idt.double_fault.set_handler_fn(exceptions::double_fault_handler)
                .set_stack_index(DOUBLE_FAULT_IST_INDEX as u16); // use a special stack for the DF handler
        }
        // reserved: 0x09 coprocessor segment overrun exception
        // missing: 0x0a invalid TSS exception
        idt.segment_not_present.set_handler_fn(exceptions::segment_not_present_handler);
        // missing: 0x0c stack segment exception
        idt.general_protection_fault.set_handler_fn(exceptions::general_protection_fault_handler);
        idt.page_fault.set_handler_fn(exceptions::page_fault_handler);
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
            idt[i].set_handler_fn(apic_unimplemented_interrupt_handler);
        }
    }

    // try to load our new IDT    
    {
        info!("trying to load IDT...");
        IDT.load();
        info!("loaded interrupt descriptor table.");
    }

    Ok(())

}


pub fn init_ap(apic_id: u8, 
               double_fault_stack_top_unusable: VirtualAddress, 
               privilege_stack_top_unusable: VirtualAddress)
               -> Result<(), &'static str> {
    info!("Setting up TSS & GDT for AP {}", apic_id);
    create_tss_gdt(apic_id, double_fault_stack_top_unusable, privilege_stack_top_unusable);


    info!("trying to load IDT for AP {}...", apic_id);
    IDT.load();
    info!("loaded IDT for AP {}.", apic_id);
    Ok(())
}


fn create_tss_gdt(apic_id: u8, 
                  double_fault_stack_top_unusable: VirtualAddress, 
                  privilege_stack_top_unusable: VirtualAddress) {
    use x86_64::instructions::segmentation::{set_cs, load_ds, load_ss};
    use x86_64::instructions::tables::load_tss;
    use x86_64::PrivilegeLevel;

    // set up TSS and get pointer to it    
    let tss_ref = {
        let mut tss = TaskStateSegment::new();
        // TSS.RSP0 is used in kernel space after a transition from Ring 3 -> Ring 0
        tss.privilege_stack_table[0] = x86_64::VirtualAddress(privilege_stack_top_unusable);
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX] = x86_64::VirtualAddress(double_fault_stack_top_unusable);

        // insert into TSS list
        TSS.insert(apic_id, tss);
        let tss_ref: &TaskStateSegment = TSS.get(apic_id).unwrap(); // safe to unwrap since we just added it to the list
        // debug!("Created TSS for apic {}, TSS: {:?}", apic_id, tss_ref);
        tss_ref
    };
    

    // set up this AP's GDT
    {
        let mut gdt = gdt::Gdt::new();

        // the following order of segments must be preserved: 
        // 0) null descriptor 
        // 1) kernel cs
        // 2) kernel ds
        // 3) user cs 32
        // 4) user ds 32
        // 5) user cs 64
        // 6) user ds 64
        // 7-8) tss
        // DO NOT rearrange the below calls to gdt.add_entry(), x86_64 has **VERY PARTICULAR** rules about this

        let kernel_cs = gdt.add_entry(gdt::Descriptor::kernel_code_segment(), PrivilegeLevel::Ring0);
        KERNEL_CODE_SELECTOR.call_once(|| kernel_cs);
        let kernel_ds = gdt.add_entry(gdt::Descriptor::kernel_data_segment(), PrivilegeLevel::Ring0);
        KERNEL_DATA_SELECTOR.call_once(|| kernel_ds);
        let user_cs_32 = gdt.add_entry(gdt::Descriptor::user_code_32_segment(), PrivilegeLevel::Ring3);
        USER_CODE_32_SELECTOR.call_once(|| user_cs_32);
        let user_ds_32 = gdt.add_entry(gdt::Descriptor::user_data_32_segment(), PrivilegeLevel::Ring3);
        USER_DATA_32_SELECTOR.call_once(|| user_ds_32);
        let user_cs_64 = gdt.add_entry(gdt::Descriptor::user_code_64_segment(), PrivilegeLevel::Ring3);
        USER_CODE_64_SELECTOR.call_once(|| user_cs_64);
        let user_ds_64 = gdt.add_entry(gdt::Descriptor::user_data_64_segment(), PrivilegeLevel::Ring3);
        USER_DATA_64_SELECTOR.call_once(|| user_ds_64);
        let tss = gdt.add_entry(gdt::Descriptor::tss_segment(tss_ref), PrivilegeLevel::Ring0);
        TSS_SELECTOR.call_once(|| tss);
        
        GDT.insert(apic_id, gdt);
        let gdt_ref = GDT.get(apic_id).unwrap(); // safe to unwrap since we just added it to the list
        gdt_ref.load();
        // debug!("Loaded GDT for apic {}: {}", apic_id, gdt_ref);
    }

    unsafe {
        set_cs(get_segment_selector(AvailableSegmentSelector::KernelCode)); // reload code segment register
        load_tss(get_segment_selector(AvailableSegmentSelector::Tss)); // load TSS
        
        load_ss(get_segment_selector(AvailableSegmentSelector::KernelData)); // unsure if necessary
        load_ds(get_segment_selector(AvailableSegmentSelector::KernelData)); // unsure if necessary
    }
}

pub fn init_handlers_apic() {
    // first, do the standard interrupt remapping, but mask all PIC interrupts / disable the PIC
    PIC.call_once( || {
        pic::ChainedPics::init(None, None, 0xFF, 0xFF) // disable all PIC IRQs
    });

    {
        let mut idt = IDT.lock(); // withholds interrupts
        
        // exceptions (IRQS from 0 -31) have already been inited before

        // fill all IDT entries with an unimplemented IRQ handler
        for i in 32..255 {
            idt[i].set_handler_fn(apic_unimplemented_interrupt_handler);
        }

        idt[0x20].set_handler_fn(apic_timer_handler);
        idt[0x21].set_handler_fn(ioapic_keyboard_handler);
        idt[apic::APIC_SPURIOUS_INTERRUPT_VECTOR as usize].set_handler_fn(apic_spurious_interrupt_handler); 


        idt[apic::TLB_SHOOTDOWN_IPI_IRQ as usize].set_handler_fn(ipi_handler);
    }


    // now it's safe to enable every LocalApic's LVT_TIMER interrupt (for scheduling)
    
}


pub fn init_handlers_pic() {
    {
        let mut idt = IDT.lock(); // withholds interrupts
		// SET UP CUSTOM INTERRUPT HANDLERS
		// we can directly index the "idt" object because it implements the Index/IndexMut traits

        // MASTER PIC starts here (0x20 - 0x27)
        idt[0x20].set_handler_fn(timer_handler);
        idt[0x21].set_handler_fn(keyboard_handler);
        
        idt[0x22].set_handler_fn(irq_0x22_handler); 
        idt[0x23].set_handler_fn(irq_0x23_handler); 
        idt[0x24].set_handler_fn(irq_0x24_handler); 
        idt[0x25].set_handler_fn(irq_0x25_handler); 
        idt[0x26].set_handler_fn(irq_0x26_handler); 

        idt[0x27].set_handler_fn(spurious_interrupt_handler); 


        // SLAVE PIC starts here (0x28 - 0x2E)        
        // idt[0x28].set_handler_fn(rtc_handler); // using the weird way temporarily

        idt[0x29].set_handler_fn(irq_0x29_handler); 
        idt[0x2A].set_handler_fn(irq_0x2A_handler); 
        idt[0x2B].set_handler_fn(irq_0x2B_handler); 
        idt[0x2C].set_handler_fn(irq_0x2C_handler); 
        idt[0x2D].set_handler_fn(irq_0x2D_handler); 

        idt[0x2E].set_handler_fn(primary_ata);
    }

    // init PIC, PIT and RTC interrupts
    let master_pic_mask: u8 = 0x0; // allow every interrupt
    let slave_pic_mask: u8 = 0b0000_1000; // everything is allowed except 0x2B 
    PIC.call_once( || {
        pic::ChainedPics::init(None, None, master_pic_mask, slave_pic_mask) // disable all PIC IRQs
    });

    pit_clock::init(CONFIG_PIT_FREQUENCY_HZ);
    let rtc_handler = rtc::init(CONFIG_RTC_FREQUENCY_HZ, rtc_interrupt_func);
    IDT.lock()[0x28].set_handler_fn(rtc_handler.unwrap());
}






/// Send an end of interrupt signal, which works for all types of interrupt chips (APIC, x2apic, PIC)
/// irq arg is only used for PIC
fn eoi(irq: Option<u8>) {
    match INTERRUPT_CHIP.load(Ordering::Acquire) {
        InterruptChip::APIC => {
            // quick fix for lockless apic eoi
            unsafe { ::core::ptr::write_volatile((::kernel_config::memory::APIC_START + 0xB0 as usize) as *mut u32, 0); }
        }
        InterruptChip::x2apic => {
            unsafe { ::x86::shared::msr::wrmsr(0x80b, 0); }
        }
        InterruptChip::PIC => {
            PIC.try().expect("IRQ 0x20: PIC not initialized").notify_end_of_interrupt(irq.expect("PIC eoi no arg provided"));
        }
    }
}



pub static mut APIC_TIMER_TICKS: usize = 0;
// 0x20
extern "x86-interrupt" fn apic_timer_handler(stack_frame: &mut ExceptionStackFrame) {
    unsafe { 
        APIC_TIMER_TICKS += 1;
        // info!(" ({}) APIC TIMER HANDLER! TICKS = {}", apic::get_my_apic_id().unwrap_or(0xFF), APIC_TIMER_TICKS);
    }
    
    eoi(None);
    // we must acknowledge the interrupt first before handling it because we context switch here, which doesn't return
    
    // if let Ok(id) = apic::get_my_apic_id() {
    //     if id == 0 {
    //         schedule!();
    //     }
    // }
    schedule!();
}

extern "x86-interrupt" fn ioapic_keyboard_handler(stack_frame: &mut ExceptionStackFrame) {
    // in this interrupt, we must read the keyboard scancode register before acknowledging the interrupt.
    let scan_code: u8 = { 
        KEYBOARD.lock().read() 
    };
	trace!("APIC KBD (AP {:?}): scan_code {:?}", apic::get_my_apic_id(), scan_code);

    keyboard::handle_keyboard_input(scan_code);	

    eoi(None);
}

extern "x86-interrupt" fn apic_spurious_interrupt_handler(stack_frame: &mut ExceptionStackFrame) {
    info!("APIC SPURIOUS INTERRUPT HANDLER!");

    eoi(None);
}

extern "x86-interrupt" fn apic_unimplemented_interrupt_handler(stack_frame: &mut ExceptionStackFrame) {
    println_unsafe!("APIC UNIMPLEMENTED IRQ!!!");
    // let all_lapics = apic::get_lapics();
    // let mut local_apic = all_lapics.get_mut(&0).expect("apic_spurious_interrupt_handler(): local_apic wasn't yet inited!");
    // let isr = local_apic.get_isr();
    // let irr = local_apic.get_irr();
    use kernel_config::memory::APIC_START;
    use core::ptr::read_volatile;
    unsafe {
        println_unsafe!("APIC ISR: {:#x} {:#x} {:#x} {:#x}, {:#x} {:#x} {:#x} {:#x} \nIRR: {:#x} {:#x} {:#x} {:#x},{:#x} {:#x} {:#x} {:#x}", 
            // ISR
            read_volatile((APIC_START + 0x100) as *const u32),
            read_volatile((APIC_START + 0x110) as *const u32),
            read_volatile((APIC_START + 0x120) as *const u32),
            read_volatile((APIC_START + 0x130) as *const u32),
            read_volatile((APIC_START + 0x140) as *const u32),
            read_volatile((APIC_START + 0x150) as *const u32),
            read_volatile((APIC_START + 0x160) as *const u32),
            read_volatile((APIC_START + 0x170) as *const u32),
            // IRR
            read_volatile((APIC_START + 0x200) as *const u32),
            read_volatile((APIC_START + 0x210) as *const u32),
            read_volatile((APIC_START + 0x220) as *const u32),
            read_volatile((APIC_START + 0x230) as *const u32),
            read_volatile((APIC_START + 0x240) as *const u32),
            read_volatile((APIC_START + 0x250) as *const u32),
            read_volatile((APIC_START + 0x260) as *const u32),
            read_volatile((APIC_START + 0x270) as *const u32)
        );

    }

    eoi(None);
}





// 0x20
extern "x86-interrupt" fn timer_handler(stack_frame: &mut ExceptionStackFrame) {
    pit_clock::handle_timer_interrupt();

	eoi(Some(0x20));
}


// 0x21
extern "x86-interrupt" fn keyboard_handler(stack_frame: &mut ExceptionStackFrame) {
    // in this interrupt, we must read the keyboard scancode register before acknowledging the interrupt.
    let scan_code: u8 = { 
        KEYBOARD.lock().read() 
    };
	// trace!("KBD: {:?}", scan_code);

    keyboard::handle_keyboard_input(scan_code);	

    eoi(Some(0x21));
}


pub static mut SPURIOUS_COUNT: u64 = 0;

/// The Spurious interrupt handler. 
/// This has given us a lot of problems on bochs emulator and on some real hardware, but not on QEMU.
/// I believe the problem is something to do with still using the antiquated PIC (instead of APIC)
/// on an SMP system with only one CPU core.
/// See here for more: https://mailman.linuxchix.org/pipermail/techtalk/2002-August/012697.html
/// Thus, for now, we will basically just ignore/ack it, but ideally this will no longer happen
/// when we transition from PIC to APIC, and disable the PIC altogether. 
extern "x86-interrupt" fn spurious_interrupt_handler(stack_frame: &mut ExceptionStackFrame ) {
    unsafe { SPURIOUS_COUNT += 1; } // cheap counter just for debug info

    if let Some(pic) = PIC.try() {
        let irq_regs = pic.read_isr_irr();
        // check if this was a real IRQ7 (parallel port) (bit 7 will be set)
        // (pretty sure this will never happen)
        // if it was a real IRQ7, we do need to ack it by sending an EOI
        if irq_regs.master_isr & 0x80 == 0x80 {
            println_unsafe!("\nGot real IRQ7, not spurious! (Unexpected behavior)");
            warn!("Got real IRQ7, not spurious! (Unexpected behavior)");
            eoi(Some(0x27));
        }
        else {
            // do nothing. Do not send an EOI.
        }
    }
    else {
        error!("spurious_interrupt_handler(): PIC wasn't initialized!");
    }

}



fn rtc_interrupt_func(rtc_ticks: Option<usize>) {
    if let Some(ticks) = rtc_ticks {      
        if (ticks % (CONFIG_TIMESLICE_PERIOD_MS * CONFIG_RTC_FREQUENCY_HZ / 1000)) == 0 {
            schedule!();
        }
    }
    else {
        error!("RTC interrupt function: unable to get RTC_TICKS system-wide state.")
    }
}

// //0x28
// extern "x86-interrupt" fn rtc_handler(stack_frame: &mut ExceptionStackFrame ) {
//     // because we use the RTC interrupt handler for context switching,
//     // we must ack the interrupt and send EOI before calling the handler, 
//     // because the handler will not return.
//     rtc::rtc_ack_irq();
//     eoi(Some(0x28));
    
//     rtc::handle_rtc_interrupt();
// }


//0x2e
extern "x86-interrupt" fn primary_ata(stack_frame:&mut ExceptionStackFrame ) {

    ata_pio::handle_primary_interrupt();

    eoi(Some(0x2e));
}


extern "x86-interrupt" fn unimplemented_interrupt_handler(stack_frame: &mut ExceptionStackFrame) {
    let irq_regs = PIC.try().map(|pic| pic.read_isr_irr());    
    println_unsafe!("UNIMPLEMENTED IRQ!!! {:?}", irq_regs);

    loop { }
}


extern "x86-interrupt" fn irq_0x22_handler(stack_frame: &mut ExceptionStackFrame) {
	let irq_regs = PIC.try().map(|pic| pic.read_isr_irr());    
    println_unsafe!("\nCaught 0x22 interrupt: {:#?}", stack_frame);
    println_unsafe!("IrqRegs: {:?}", irq_regs);

    loop { }
}

extern "x86-interrupt" fn irq_0x23_handler(stack_frame: &mut ExceptionStackFrame) {
    let irq_regs = PIC.try().map(|pic| pic.read_isr_irr());  
	println_unsafe!("\nCaught 0x23 interrupt: {:#?}", stack_frame);
    println_unsafe!("IrqRegs: {:?}", irq_regs);

    loop { }
}

extern "x86-interrupt" fn irq_0x24_handler(stack_frame: &mut ExceptionStackFrame) {
	let irq_regs = PIC.try().map(|pic| pic.read_isr_irr());
    println_unsafe!("\nCaught 0x24 interrupt: {:#?}", stack_frame);
    println_unsafe!("IrqRegs: {:?}", irq_regs);

    loop { }
}

extern "x86-interrupt" fn irq_0x25_handler(stack_frame: &mut ExceptionStackFrame) {
	let irq_regs = PIC.try().map(|pic| pic.read_isr_irr());  
    println_unsafe!("\nCaught 0x25 interrupt: {:#?}", stack_frame);
    println_unsafe!("IrqRegs: {:?}", irq_regs);

    loop { }
}


extern "x86-interrupt" fn irq_0x26_handler(stack_frame: &mut ExceptionStackFrame) {
	let irq_regs = PIC.try().map(|pic| pic.read_isr_irr());  
    println_unsafe!("\nCaught 0x26 interrupt: {:#?}", stack_frame);
    println_unsafe!("IrqRegs: {:?}", irq_regs);

    loop { }
}

extern "x86-interrupt" fn irq_0x29_handler(stack_frame: &mut ExceptionStackFrame) {
	let irq_regs = PIC.try().map(|pic| pic.read_isr_irr());  
    println_unsafe!("\nCaught 0x29 interrupt: {:#?}", stack_frame);
    println_unsafe!("IrqRegs: {:?}", irq_regs);

    loop { }
}



extern "x86-interrupt" fn irq_0x2A_handler(stack_frame: &mut ExceptionStackFrame) {
	let irq_regs = PIC.try().map(|pic| pic.read_isr_irr());  
    println_unsafe!("\nCaught 0x2A interrupt: {:#?}", stack_frame);
    println_unsafe!("IrqRegs: {:?}", irq_regs);

    loop { }
}


extern "x86-interrupt" fn irq_0x2B_handler(stack_frame: &mut ExceptionStackFrame) {
	let irq_regs = PIC.try().map(|pic| pic.read_isr_irr());  
    println_unsafe!("\nCaught 0x2B interrupt: {:#?}", stack_frame);
    println_unsafe!("IrqRegs: {:?}", irq_regs);

    loop { }
}


extern "x86-interrupt" fn irq_0x2C_handler(stack_frame: &mut ExceptionStackFrame) {
	let irq_regs = PIC.try().map(|pic| pic.read_isr_irr());  
    println_unsafe!("\nCaught 0x2C interrupt: {:#?}", stack_frame);
    println_unsafe!("IrqRegs: {:?}", irq_regs);

    loop { }
}


extern "x86-interrupt" fn irq_0x2D_handler(stack_frame: &mut ExceptionStackFrame) {
	let irq_regs = PIC.try().map(|pic| pic.read_isr_irr());  
    println_unsafe!("\nCaught 0x2D interrupt: {:#?}", stack_frame);
    println_unsafe!("IrqRegs: {:?}", irq_regs);

    loop { }
}



extern "x86-interrupt" fn ipi_handler(stack_frame: &mut ExceptionStackFrame) {
    trace!("ipi_handler (AP {})", apic::get_my_apic_id().unwrap_or(0xFF));
    apic::handle_tlb_shootdown_ipi();

    eoi(None);
}

