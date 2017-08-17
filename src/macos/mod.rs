//! The MacOS implementation of the [SharedLibrary
//! trait](../trait.SharedLibrary.html).

use super::{Bias, IterationControl, NamedMemoryRange, Svma};
use super::EhFrameHdr as EhFrameHdrTrait;
use super::EhFrame as EhFrameTrait;
use super::Segment as SegmentTrait;
use super::SharedLibrary as SharedLibraryTrait;

use std::ffi::CStr;
use std::marker::PhantomData;
use std::isize;
use std::ptr;
use std::sync::Mutex;
use std::usize;

mod bindings;

lazy_static! {
    /// A lock protecting dyld FFI calls.
    ///
    /// MacOS does not provide an atomic way to iterate shared libraries, so
    /// *you* must take this lock whenever dynamically adding or removing shared
    /// libraries to ensure that there are no races with iterating shared
    /// libraries.
    pub static ref DYLD_LOCK: Mutex<()> = Mutex::new(());
}

/// TODO FITZGEN
#[derive(Debug)]
pub struct EhFrameHdr<'a>(Section<'a>);

impl<'a> EhFrameHdrTrait for EhFrameHdr<'a> {
    type Segment = Segment<'a>;
    type SharedLibrary = SharedLibrary<'a>;
    type EhFrame = EhFrame<'a>;
}

impl<'a> NamedMemoryRange<SharedLibrary<'a>> for EhFrameHdr<'a> {
    fn name(&self) -> &CStr {
        self.0.name()
    }

    fn stated_virtual_memory_address(&self) -> Svma {
        self.0.stated_virtual_memory_address()
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

/// TODO FITZGEN
#[derive(Debug)]
pub struct EhFrame<'a>(Section<'a>);

impl<'a> EhFrameTrait for EhFrame<'a> {
    type Segment = Segment<'a>;
    type SharedLibrary = SharedLibrary<'a>;
    type EhFrameHdr = EhFrameHdr<'a>;
}

impl<'a> NamedMemoryRange<SharedLibrary<'a>> for EhFrame<'a> {
    fn name(&self) -> &CStr {
        self.0.name()
    }

    fn stated_virtual_memory_address(&self) -> Svma {
        self.0.stated_virtual_memory_address()
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

/// A Mach-O section mapped into memory somewhere within a Mach-O segment.
#[derive(Debug)]
#[allow(missing_docs)]
pub enum Section<'a> {
    Section32(&'a bindings::section),
    Section64(&'a bindings::section_64),
}

impl<'a> NamedMemoryRange<SharedLibrary<'a>> for Section<'a> {
    #[inline]
    fn name(&self) -> &CStr {
        match *self {
            Section::Section32(s) => unsafe { CStr::from_ptr(s.sectname.as_ptr()) },
            Section::Section64(s) => unsafe { CStr::from_ptr(s.sectname.as_ptr()) },
        }
    }

    #[inline]
    fn stated_virtual_memory_address(&self) -> Svma {
        match *self {
            Section::Section32(s) => Svma(s.addr as usize as *const u8),
            Section::Section64(s) => {
                assert!(s.addr < usize::MAX as u64);
                Svma(s.addr as usize as *const u8)
            }
        }
    }

    #[inline]
    fn len(&self) -> usize {
        match *self {
            Section::Section32(s) => s.size as usize,
            Section::Section64(s) => s.size as usize,
        }
    }
}

/// An iterator over Mach-O sections.
#[derive(Debug)]
#[allow(missing_docs)]
pub enum SectionIter<'a> {
    SectionIter32 {
        sections: *const bindings::section,
        num_sections: usize,
        segment: PhantomData<&'a Segment<'a>>,
    },
    SectionIter64 {
        sections: *const bindings::section_64,
        num_sections: usize,
        segment: PhantomData<&'a Segment<'a>>,
    }
}

impl<'a> Iterator for SectionIter<'a> {
    type Item = Section<'a>;

    #[inline]
    fn next(&mut self) -> Option<Section<'a>> {
        match *self {
            SectionIter::SectionIter32 { ref mut sections, ref mut num_sections, .. } => {
                if *num_sections == 0 {
                    return None;
                }

                *num_sections -= 1;
                unsafe {
                    let section = (*sections).as_ref().unwrap();
                    *sections = (*sections).offset(1);
                    Some(Section::Section32(section))
                }
            }
            SectionIter::SectionIter64 { ref mut sections, ref mut num_sections, .. } => {
                if *num_sections == 0 {
                    return None;
                }

                *num_sections -= 1;
                unsafe {
                    let section = (*sections).as_ref().unwrap();
                    *sections = (*sections).offset(1);
                    Some(Section::Section64(section))
                }
            }
        }
    }
}

/// A Mach-O segment.
#[derive(Debug)]
pub enum Segment<'a> {
    /// A 32-bit Mach-O segment.
    Segment32(&'a bindings::segment_command),
    /// A 64-bit Mach-O segment.
    Segment64(&'a bindings::segment_command_64),
}

impl<'a> SegmentTrait for Segment<'a> {
    type EhFrameHdr = EhFrameHdr<'a>;
    type SharedLibrary = SharedLibrary<'a>;
    type EhFrame = EhFrame<'a>;
}

impl<'a> NamedMemoryRange<SharedLibrary<'a>> for Segment<'a> {
    #[inline]
    fn name(&self) -> &CStr {
        match *self {
            Segment::Segment32(seg) => unsafe { CStr::from_ptr(seg.segname.as_ptr()) },
            Segment::Segment64(seg) => unsafe { CStr::from_ptr(seg.segname.as_ptr()) },
        }
    }

    #[inline]
    fn stated_virtual_memory_address(&self) -> Svma {
        match *self {
            Segment::Segment32(seg) => Svma(seg.vmaddr as usize as *const u8),
            Segment::Segment64(seg) => {
                assert!(seg.vmaddr <= (usize::MAX as u64));
                Svma(seg.vmaddr as usize as *const u8)
            }
        }
    }

    #[inline]
    fn len(&self) -> usize {
        match *self {
            Segment::Segment32(seg) => seg.vmsize as usize,
            Segment::Segment64(seg) => {
                assert!(seg.vmsize <= (usize::MAX as u64));
                seg.vmsize as usize
            }
        }
    }
}

impl<'a> Segment<'a> {
    /// TODO FITZGEN
    #[inline]
    pub fn sections(&self) -> SectionIter<'a> {
        match *self {
            Segment::Segment32(seg) => {
                let sections = unsafe {
                    (seg as *const bindings::segment_command).offset(1)
                };
                let sections = sections as *const bindings::section;
                SectionIter::SectionIter32 {
                    sections,
                    num_sections: seg.nsects as usize,
                    segment: PhantomData,
                }
            }
            Segment::Segment64(seg) => {
                let sections = unsafe {
                    (seg as *const bindings::segment_command_64).offset(1)
                };
                let sections = sections as *const bindings::section_64;
                SectionIter::SectionIter64 {
                    sections,
                    num_sections: seg.nsects as usize,
                    segment: PhantomData,
                }
            }
        }
    }
}

/// An iterator over Mach-O segments.
#[derive(Debug)]
pub struct SegmentIter<'a> {
    phantom: PhantomData<&'a SharedLibrary<'a>>,
    commands: *const bindings::load_command,
    num_commands: usize,
}

impl<'a> Iterator for SegmentIter<'a> {
    type Item = Segment<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.num_commands > 0 {
            self.num_commands -= 1;

            let this_command = unsafe { self.commands.as_ref().unwrap() };
            let command_size = this_command.cmdsize as isize;

            match this_command.cmd {
                bindings::LC_SEGMENT => {
                    let segment = self.commands as *const bindings::segment_command;
                    let segment = unsafe { segment.as_ref().unwrap() };
                    self.commands =
                        unsafe { (self.commands as *const u8).offset(command_size) as *const _ };
                    return Some(Segment::Segment32(segment));
                }
                bindings::LC_SEGMENT_64 => {
                    let segment = self.commands as *const bindings::segment_command_64;
                    let segment = unsafe { segment.as_ref().unwrap() };
                    self.commands =
                        unsafe { (self.commands as *const u8).offset(command_size) as *const _ };
                    return Some(Segment::Segment64(segment));
                }
                _ => {
                    // Some other kind of load command; skip to the next one.
                    self.commands =
                        unsafe { (self.commands as *const u8).offset(command_size) as *const _ };
                    continue;
                }
            }
        }

        None
    }
}

#[derive(Debug)]
enum MachType {
    Mach32,
    Mach64,
}

impl MachType {
    unsafe fn from_header_ptr(header: *const bindings::mach_header) -> Option<MachType> {
        header.as_ref().and_then(|header| {
            match header.magic {
                bindings::MH_MAGIC => Some(MachType::Mach32),
                bindings::MH_MAGIC_64 => Some(MachType::Mach64),
                _ => None,
            }
        })
    }
}

#[derive(Debug)]
enum MachHeader<'a> {
    Header32(&'a bindings::mach_header),
    Header64(&'a bindings::mach_header_64),
}

impl<'a> MachHeader<'a> {
    unsafe fn from_header_ptr(header: *const bindings::mach_header) -> Option<MachHeader<'a>> {
        MachType::from_header_ptr(header).and_then(|ty| {
            match ty {
                MachType::Mach32 => header.as_ref().map(MachHeader::Header32),
                MachType::Mach64 => (header as *const _).as_ref().map(MachHeader::Header64),
            }
        })
    }
}

/// The MacOS implementation of the [SharedLibrary
/// trait](../trait.SharedLibrary.html).
///
/// This wraps the `_dyld_image_count` and
/// `_dyld_get_image_{header,vmaddr_slide,name}` system APIs from the
/// `<mach-o/dyld.h>` header.
#[derive(Debug)]
pub struct SharedLibrary<'a> {
    header: MachHeader<'a>,
    slide: isize,
    name: &'a CStr,
}

impl<'a> SharedLibrary<'a> {
    fn new(header: MachHeader<'a>, slide: isize, name: &'a CStr) -> Self {
        SharedLibrary {
            header: header,
            slide: slide,
            name: name,
        }
    }

    /// TODO FITZGEN
    pub fn sections(&self) -> AllSectionsIter<'a> {
        AllSectionsIter {
            segments: self.segments(),
            sections: None,
        }
    }
}

/// An iterator over all the sections that are in all mapped segments in a
/// Mach-O shared library.
pub struct AllSectionsIter<'a> {
    segments: SegmentIter<'a>,
    sections: Option<SectionIter<'a>>,
}

impl<'a> Iterator for AllSectionsIter<'a> {
    type Item = Section<'a>;

    #[inline]
    fn next(&mut self) -> Option<Section<'a>> {
        // AFAIK, there is no faster way to iterate all Mach-O sections, than to
        // iterate Mach-O segments and then iterate over the sections in each
        // segment.
        loop {
            if let Some(section) = self.sections.as_mut().and_then(|sections| sections.next()) {
                return Some(section);
            }

            self.sections = self.segments.next().map(|seg| seg.sections());
            if self.sections.is_none() {
                return None;
            }
        }
    }
}

impl<'a> SharedLibraryTrait for SharedLibrary<'a> {
    type Segment = Segment<'a>;
    type SegmentIter = SegmentIter<'a>;
    type EhFrameHdr = EhFrameHdr<'a>;
    type EhFrame = EhFrame<'a>;

    #[inline]
    fn name(&self) -> &CStr {
        self.name
    }

    #[inline]
    fn segments(&self) -> Self::SegmentIter {
        match self.header {
            MachHeader::Header32(header) => {
                let num_commands = header.ncmds;
                let header = header as *const bindings::mach_header;
                let commands = unsafe { header.offset(1) as *const bindings::load_command };
                SegmentIter {
                    phantom: PhantomData,
                    commands: commands,
                    num_commands: num_commands as usize,
                }
            }
            MachHeader::Header64(header) => {
                let num_commands = header.ncmds;
                let header = header as *const bindings::mach_header_64;
                let commands = unsafe { header.offset(1) as *const bindings::load_command };
                SegmentIter {
                    phantom: PhantomData,
                    commands: commands,
                    num_commands: num_commands as usize,
                }
            }
        }
    }

    #[inline]
    fn eh_frame_hdr(&self) -> Option<Self::EhFrameHdr> {
        const EH_FRAME_HDR_NAME: &'static [u8; 14] = b"__eh_frame_hdr";

        self.sections()
            .find(|s| s.name().to_bytes() == &EH_FRAME_HDR_NAME[..])
            .map(|s| EhFrameHdr(s))
    }

    #[inline]
    fn eh_frame(&self) -> Option<Self::EhFrame> {
        const EH_FRAME_NAME: &'static [u8; 10] = b"__eh_frame";

        self.sections()
            .find(|s| s.name().to_bytes() == &EH_FRAME_NAME[..])
            .map(|s| EhFrame(s))
    }

    #[inline]
    fn virtual_memory_bias(&self) -> Bias {
        Bias(self.slide)
    }

    fn each<F, C>(mut f: F)
        where F: FnMut(&Self) -> C,
              C: Into<IterationControl>
    {
        // Make sure we have exclusive access to dyld so that (hopefully) no one
        // else adds or removes shared libraries while we are iterating them.
        let _dyld_lock = DYLD_LOCK.lock();

        let count = unsafe { bindings::_dyld_image_count() };

        for image_idx in 0..count {
            let (header, slide, name) = unsafe {
                (bindings::_dyld_get_image_header(image_idx),
                 bindings::_dyld_get_image_vmaddr_slide(image_idx),
                 bindings::_dyld_get_image_name(image_idx))
            };

            if let Some(header) = unsafe { MachHeader::from_header_ptr(header) } {
                assert!(slide != 0,
                        "If we have a header pointer, slide should be valid");
                assert!(name != ptr::null(),
                        "If we have a header pointer, name should be valid");

                let name = unsafe { CStr::from_ptr(name) };
                let shlib = SharedLibrary::new(header, slide, name);

                match f(&shlib).into() {
                    IterationControl::Break => break,
                    IterationControl::Continue => continue,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use macos;
    use super::super::{IterationControl, NamedMemoryRange, SharedLibrary};

    #[test]
    fn have_eh_frame_section() {
        let mut found_eh_frame_in_all_sections = false;
        let mut found_eh_frame_in_segment_sections = false;

        macos::SharedLibrary::each(|shlib| {
            for section in shlib.sections() {
                found_eh_frame_in_all_sections |= section.name().to_string_lossy() == "__eh_frame";
            }

            for segment in shlib.segments() {
                for section in segment.sections() {
                    found_eh_frame_in_segment_sections |= section.name().to_string_lossy() == "__eh_frame";
                }
            }
        });

        assert!(found_eh_frame_in_all_sections);
        assert!(found_eh_frame_in_segment_sections)
    }

    #[test]
    fn have_libdyld() {
        let mut found_dyld = false;
        macos::SharedLibrary::each(|shlib| {
            found_dyld |= shlib.name
                .to_bytes()
                .split(|c| *c == b'.' || *c == b'/')
                .find(|s| s == b"libdyld")
                .is_some();
        });
        assert!(found_dyld);
    }

    #[test]
    fn can_break() {
        let mut first_count = 0;
        macos::SharedLibrary::each(|_| {
            first_count += 1;
        });
        assert!(first_count > 2);

        let mut second_count = 0;
        macos::SharedLibrary::each(|_| {
            second_count += 1;

            if second_count == first_count - 1 {
                IterationControl::Break
            } else {
                IterationControl::Continue
            }
        });
        assert_eq!(second_count, first_count - 1);
    }

    #[test]
    fn get_name() {
        macos::SharedLibrary::each(|shlib| {
            let _ = shlib.name();
        });
    }

    #[test]
    fn have_text_or_pagezero() {
        macos::SharedLibrary::each(|shlib| {
            println!("shlib = {:?}", shlib.name());

            let mut found_text_or_pagezero = false;
            for seg in shlib.segments() {
                println!("    segment = {:?}", seg.name());

                found_text_or_pagezero |= seg.name().to_bytes() == b"__TEXT";
                found_text_or_pagezero |= seg.name().to_bytes() == b"__PAGEZERO";
            }
            assert!(found_text_or_pagezero);
        });
    }
}
