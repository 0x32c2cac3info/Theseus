use port_io::Port;
use spin::Mutex; 
use core::sync::atomic::{AtomicUsize, Ordering};
use interrupts::pit_clock;




//data written here sets information at CONFIG_DATA
const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

//drive select port for primary bus (bus 0)
const BUS_SELECT_PRIMARY: u16 = 0x1F6;
//set "DRIVE_SELECT" port to choose master or slave drive
const IDENTIFY_MASTER_DRIVE: u16 = 0xA0;
const IDENTIFY_SLAVE_DRIVE: u16 = 0xB0;

const IDENTIFY_COMMAND: u16 = 0xEC;

const READ_MASTER: u16 = 0xE0;



//access to CONFIG_ADDRESS 
static PCI_CONFIG_ADDRESS_PORT: Mutex<Port<u32>> = Mutex::new( Port::new(CONFIG_ADDRESS));
//acccess to CONFIG_DATA
static PCI_CONFIG_DATA_PORT: Mutex<Port<u32>> = Mutex::new( Port::new(CONFIG_DATA));

//ports used in IDENTIFY command 
static PRIMARY_BUS_IO: Mutex<Port<u8>> = Mutex::new( Port::new(BUS_SELECT_PRIMARY));
static SECTORCOUNT: Mutex<Port<u8>> = Mutex::new( Port::new(0x1F2));
static LBALO: Mutex<Port<u8>> = Mutex::new( Port::new(0x1F3));
static LBAMID: Mutex<Port<u8>> = Mutex::new( Port::new(0x1F4));
static LBAHI: Mutex<Port<u8>> = Mutex::new( Port::new(0x1F5));
static COMMAND_IO: Mutex<Port<u8>> = Mutex::new( Port::new(0x1F7));
static PRIMARY_DATA_PORT: Mutex<Port<u16>> = Mutex::new( Port::new(0x1F0));



//used to read from PCI config, additionally initializes PCI buses to be used
//might be better to set input paramters as u8 (method used in osdev)
pub fn pciConfigRead(bus: u32, slot: u32, func: u32, offset: u32)->u16{
    
    //data to be written to CONFIG_ADDRESS
    let address:u32 = ((bus<<16) | (slot<<11) |  (func << 8) | (offset&0xfc) | 0x80000000);

    unsafe{PCI_CONFIG_ADDRESS_PORT.lock().write(address);}

    ((PCI_CONFIG_DATA_PORT.lock().read() >> (offset&2) * 8) & 0xffff)as u16

}

//reads two bytes from the 
pub fn read_primary_data_port()-> [u16; 256]{
    let mut arr: [u16; 256] = [0;256];
	
	for word in 0..256{
    	while((COMMAND_IO.lock().read()>>3)%2 ==0){trace!("stuck in read_primary_data_port function")}
		arr[word] = PRIMARY_DATA_PORT.lock().read();

    }
	
    arr

}

//returns ATA identify information 
pub fn ATADriveExists(drive:u8)-> AtaIdentifyData{
    
    let mut command_value: u8 = COMMAND_IO.lock().read();
    //let mut arr: [u16; 256] = [0; 256];
    //set port values for bus 0 to detect ATA device 
    unsafe{PRIMARY_BUS_IO.lock().write(drive);
           
           SECTORCOUNT.lock().write(0);
           LBALO.lock().write(0);
           LBAMID.lock().write(0);
           LBAHI.lock().write(0);

           COMMAND_IO.lock().write(0xEC);


    }

	
    command_value = COMMAND_IO.lock().read();
    //if value is 0, no drive exists
    if command_value == 0{
        trace!("No Drive Exists");
    }
    
    
    //wait for update-in-progress value (bit 7 of COMMAND_IO port) to be set to 0
    command_value =(COMMAND_IO.lock().read());
    while ((command_value>>7)%2 != 0)  {
        //trace to debug and view value being received
        trace!("{}: update-in-progress in disk drive COMMAND_IO bit 7 not cleared", command_value);
        command_value = (COMMAND_IO.lock().read());
    }
    
    
    //if LBAhi or LBAlo values at this point are nonzero, drive is not ATA compatible
    if LBAMID.lock().read() != 0 || LBAHI.lock().read() !=0 {
        trace!("mid or hi LBA not set to 0 when it should be");
    }
    
	//waits for error bit or data ready bit to set
    command_value = COMMAND_IO.lock().read();
    while((command_value>>3)%2 ==0 && command_value%2 == 0){
        trace!("{} is bit 0 of COMMAND_IO which should be cleared, {} is bit 6 which should be set",command_value, command_value>>3);
        command_value = COMMAND_IO.lock().read();
    }

	if command_value%2 == 1{
		let identify_data = AtaIdentifyData{..Default::default()};
		return identify_data;

	}
    


	let identify_data = AtaIdentifyData::new(read_primary_data_port()); 
    identify_data 
    
}

//read from disk at address input 
pub fn pio_read(lba:u32)->[u16; 256]{

    //selects master drive(using 0xE0 value) in primary bus (by writing to PRIMARY_BUS_IO-port 0x1F6)
    let master_select: u8 = 0xE0 | (0 << 4) | ((lba >> 24) & 0x0F) as u8;
    unsafe{PRIMARY_BUS_IO.lock().write(master_select);

    SECTORCOUNT.lock().write(0);

    //lba is written into disk 
    LBALO.lock().write((lba&0xFF)as u8);
    //trace!("{} here",lba>>8&0xFF);
    LBAMID.lock().write((lba>>8 &0xFF)as u8);
    LBAHI.lock().write((lba>>16 &0xFF)as u8);

    COMMAND_IO.lock().write(0x20);
    }


    //just returning this during testing to make sure program compiles
    //return COMMAND_IO.lock().read()>>3
	trace!("got to end of pio_read function");
	
    read_primary_data_port()



}

//exists to handle interrupts from PCI
//could be used later to replace polling system with interrupt system for reading and writing
pub fn handle_primary_interrupt(){
    trace!("Got IRQ 14!");
}


//AtaIdentifyData struct and implemenations from Tifflin Kernel
#[repr(C,packed)]
pub struct AtaIdentifyData
{
	pub flags: u16,
	_unused1: [u16; 9],
	pub serial_number: [u8; 20],
	_unused2: [u16; 3],
	pub firmware_ver: [u8; 8],
	pub model_number: [u8; 40],
	/// Maximum number of blocks per transfer
	pub sect_per_int: u16,
	_unused3: u16,
	pub capabilities: [u16; 2],
	_unused4: [u16; 2],
	/// Bitset of translation fields (next five shorts)
	pub valid_ext_data: u16,
	_unused5: [u16; 5],
	pub size_of_rw_multiple: u16,
	/// LBA 28 sector count (if zero, use 48)
	pub sector_count_28: u32,
	_unused6: [u16; 100-62],
	/// LBA 48 sector count
	pub sector_count_48: u64,
	_unused7: [u16; 2],
	/// [0:3] Physical sector size (in logical sectors
	pub physical_sector_size: u16,
	_unused8: [u16; 9],
	/// Number of words per logical sector
	pub words_per_logical_sector: u32,
	_unusedz: [u16; 257-119],
}

impl Default for AtaIdentifyData {
	fn default() -> AtaIdentifyData {
		// SAFE: Plain old data
		unsafe { ::core::mem::zeroed() }
	}

}


impl AtaIdentifyData{

	//takes an array storing data from ATA IDENTIFY command and returns struct with the relevant information
	fn new(arr: [u16; 256])-> AtaIdentifyData{

		//transmutes the array of u16s from the ATA device into an ATAIdentifyData struct
		let mut identify_data: AtaIdentifyData =unsafe {::core::mem::transmute(arr)};
		flip_bytes(&mut identify_data.serial_number);
		flip_bytes(&mut identify_data.firmware_ver);
		flip_bytes(&mut identify_data.model_number);


		return identify_data

	}
	

}


//used to print ATAIdentifyData information to console
impl ::core::fmt::Display for AtaIdentifyData {
	fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result
	{
		write!(f, "AtaIdentifyData {{");
		write!(f, " flags: {:#x}", self.flags);
		write!(f, " serial_number: {:?}", RawString(&self.serial_number));
		write!(f, " firmware_ver: {:?}", RawString(&self.firmware_ver));
		write!(f, " model_number: {:?}", RawString(&self.model_number));
		write!(f, " sect_per_int: {}", self.sect_per_int & 0xFF);
		write!(f, " capabilities: [{:#x},{:#x}]", self.capabilities[0], self.capabilities[1]);
		write!(f, " valid_ext_data: {}", self.valid_ext_data);
		write!(f, " size_of_rw_multiple: {}", self.size_of_rw_multiple);
		write!(f, " sector_count_28: {:#x}", self.sector_count_28);
		write!(f, " sector_count_48: {:#x}", self.sector_count_48);
		write!(f, " physical_sector_size: {}", self.physical_sector_size);
		write!(f, " words_per_logical_sector: {}", self.words_per_logical_sector);
		write!(f, "}}");
		Ok( () )
	}
}

//flips pairs of bytes, helpful for transfers between certain big-endian and little-endian interfaces 
fn flip_bytes(bytes: &mut [u8]) {
	for pair in bytes.chunks_mut(2) {
		pair.swap(0, 1);
	}
}

//prints basic ASCII characters to the console
pub struct RawString<'a>(pub &'a [u8]);
impl<'a> ::core::fmt::Debug for RawString<'a>
{
	fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result
	{
		try!(write!(f, "b\""));
		for &b in self.0
		{
			match b
			{
			b'\\' => try!(write!(f, "\\\\")),
			b'\n' => try!(write!(f, "\\n")),
			b'\r' => try!(write!(f, "\\r")),
			b'"' => try!(write!(f, "\\\"")),
			b'\0' => try!(write!(f, "\\0")),
			// ASCII printable characters
			32...127 => try!(write!(f, "{}", b as char)),
			_ => try!(write!(f, "\\x{:02x}", b)),
			}
		}
		try!(write!(f, "\""));
		::core::result::Result::Ok( () )
	}
}