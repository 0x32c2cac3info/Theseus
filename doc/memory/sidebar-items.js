window.SIDEBAR_ITEMS = {"constant":[["PAGE_SIZE","Page size is 4096 bytes, 4KiB pages."]],"enum":[["MemoryRegionType","Types of physical memory. See each variant’s documentation."],["UnmapResult","The frames returned from the action of unmapping a page table entry. See the `PageTableEntry::set_unmapped()` function."]],"fn":[["allocate_frames","Allocates the given number of frames with no constraints on the starting physical address."],["allocate_frames_at","Allocates the given number of frames starting at (inclusive of) the frame containing the given `PhysicalAddress`."],["allocate_frames_by_bytes","Allocates frames with no constraints on the starting physical address,  with a size given by the number of bytes. "],["allocate_frames_by_bytes_at","Allocates frames starting at the given `PhysicalAddress` with a size given in number of bytes. "],["allocate_frames_by_bytes_deferred","Similar to `allocated_frames_deferred()`, but accepts a size value for the allocated frames in number of bytes instead of number of frames. "],["allocate_frames_deferred","The core frame allocation routine that allocates the given number of physical frames, optionally at the requested starting `PhysicalAddress`."],["allocate_pages","Allocates the given number of pages with no constraints on the starting virtual address."],["allocate_pages_at","Allocates the given number of pages starting at (inclusive of) the page containing the given `VirtualAddress`."],["allocate_pages_by_bytes","Allocates pages with no constraints on the starting virtual address,  with a size given by the number of bytes. "],["allocate_pages_by_bytes_at","Allocates pages starting at the given `VirtualAddress` with a size given in number of bytes. "],["allocate_pages_by_bytes_deferred","Similar to `allocated_pages_deferred()`, but accepts a size value for the allocated pages in number of bytes instead of number of pages. "],["allocate_pages_deferred","The core page allocation routine that allocates the given number of virtual pages, optionally at the requested starting `VirtualAddress`."],["create_contiguous_mapping","A convenience function that creates a new memory mapping by allocating frames that are contiguous in physical memory. If contiguous frames are not required, then see `create_mapping()`. Returns a tuple containing the new `MappedPages` and the starting PhysicalAddress of the first frame, which is a convenient way to get the physical address without walking the page tables."],["create_mapping","A convenience function that creates a new memory mapping. The pages allocated are contiguous in memory but there’s no guarantee that the frames they are mapped to are also contiguous in memory. If contiguous frames are required then see `create_contiguous_mapping()`. Returns the new `MappedPages.` "],["get_current_p4","Returns the current top-level (P4) root page table frame."],["get_kernel_mmi_ref","Returns a reference to the kernel’s `MemoryManagementInfo`, if initialized. If not, it returns `None`."],["init","Initializes the virtual memory management system. Consumes the given BootInformation, because after the memory system is initialized, the original BootInformation will be unmapped and inaccessible."],["init_post_heap","Finishes initializing the virtual memory management system after the heap is initialized and returns a MemoryManagementInfo instance, which represents the initial (the kernel’s) address space. "],["set_broadcast_tlb_shootdown_cb","Set the function callback that will be invoked every time a TLB shootdown is necessary, i.e., during page table remapping and unmapping operations."]],"static":[["BROADCAST_TLB_SHOOTDOWN_FUNC",""]],"struct":[["AggregatedSectionMemoryBounds","The address bounds and flags of the initial kernel sections that need mapping. "],["AllocatedFrames","Represents a range of allocated `PhysicalAddress`es, specified in `Frame`s. "],["AllocatedPages","Represents a range of allocated `VirtualAddress`es, specified in `Page`s. "],["DeferredAllocAction","A series of pending actions related to page allocator bookkeeping, which may result in heap allocation. "],["EntryFlags","Page table entry flags on the x86_64 architecture. "],["Frame","A `Frame` is a chunk of physical memory aligned to a [`PAGE_SIZE`] boundary."],["FrameRange","A range of [`Frame`]s that are contiguous in physical memory."],["MappedPages","Represents a contiguous range of virtual memory pages that are currently mapped.  A `MappedPages` object can only have a single range of contiguous pages, not multiple disjoint ranges. This does not guarantee that its pages are mapped to frames that are contiguous in physical memory."],["Mapper",""],["MemoryManagementInfo","This holds all the information for a `Task`’s memory mappings and address space (this is basically the equivalent of Linux’s mm_struct)"],["Page","A `Page` is a chunk of virtual memory aligned to a [`PAGE_SIZE`] boundary."],["PageRange","A range of [`Page`]s that are contiguous in virtual memory."],["PageTable","A top-level root (P4) page table."],["PageTableEntry","A page table entry, which is a `u64` value under the hood."],["PhysicalAddress","A physical memory address, which is a `usize` under the hood."],["PhysicalMemoryRegion","A region of physical memory."],["SectionMemoryBounds","The address bounds and mapping flags of a section’s memory region."],["TemporaryPage","A page that can be temporarily mapped to the recursive page table frame, used for purposes of editing a top-level (P4) page table itself."],["UnmappedFrames","A range of frames that have been unmapped from a `PageTableEntry` that previously mapped that frame exclusively (i.e., “owned it”)."],["VirtualAddress","A virtual memory address, which is a `usize` under the hood."]],"type":[["MmiRef","A shareable reference to a `MemoryManagementInfo` struct wrapper in a lock."]]};