use crate::{
    AssemblyLoader,
    error::AssemblyLoadError,
    loader::{SUPPORT_ASSEMBLY, SUPPORT_LIBRARY},
};
use dotnet_types::{TypeDescription, resolution::ResolutionS};
use dotnetdll::prelude::*;
use std::ptr;

impl AssemblyLoader {
    pub(crate) fn add_support_library(&mut self) -> Result<(), AssemblyLoadError> {
        // Ensure alignment for SUPPORT_LIBRARY
        let len = SUPPORT_LIBRARY.len();
        let cap = len.div_ceil(8);
        let mut aligned: Vec<u64> = vec![0u64; cap];
        // SAFETY: The source and destination are both valid for 'len' bytes.
        // The destination buffer 'aligned' has enough capacity as it's allocated with 'div_ceil(8)'.
        // Both buffers are non-overlapping as 'aligned' is newly allocated.
        unsafe {
            ptr::copy_nonoverlapping(
                SUPPORT_LIBRARY.as_ptr(),
                aligned.as_mut_ptr() as *mut u8,
                len,
            );
        }
        let aligned_boxed = aligned.into_boxed_slice();
        let aligned_ptr = Box::into_raw(aligned_boxed);
        // SAFETY: We manually track this leaked box in MetadataOwner to reclaim it on drop.
        // It's safe to treat as 'static because AssemblyLoader owns MetadataOwner and
        // ensures it lives long enough.
        let aligned_slice: &'static mut [u64] = unsafe { &mut *aligned_ptr };
        unsafe {
            self.metadata.get_mut().add_u64_slice(aligned_ptr);
        }

        let byte_slice =
            // SAFETY: 'aligned_slice' is a leaked Box<[u64]> which is valid for its entire length.
            // Converting it to a *const u8 slice of length 'len' is safe because it was initialized
            // with exactly 'len' bytes from SUPPORT_LIBRARY.
            unsafe { std::slice::from_raw_parts(aligned_slice.as_ptr() as *const u8, len) };

        let support_res_raw =
            Resolution::parse(byte_slice, ReadOptions::default()).map_err(|e| {
                AssemblyLoadError::InvalidFormat(format!("failed to parse support library: {}", e))
            })?;
        let support_res_box = Box::new(support_res_raw);
        let support_res_ptr = Box::into_raw(support_res_box);
        // SAFETY: We manually track this leaked box in MetadataOwner to reclaim it on drop.
        let support_res: &'static mut Resolution<'static> = unsafe { &mut *support_res_ptr };
        unsafe {
            self.metadata.get_mut().add_resolution(support_res_ptr);
        }

        {
            let mut external = self.external.write();
            external.insert(
                SUPPORT_ASSEMBLY.to_string(),
                Some(ResolutionS::new(support_res)),
            );
        }

        for (index, t) in support_res.type_definitions.iter().enumerate() {
            for a in &t.attributes {
                // the target stub attribute is internal to the support library,
                // so the constructor reference will always be a Definition variant
                let parent = match a.constructor {
                    UserMethod::Definition(d) => &support_res[d.parent_type()],
                    UserMethod::Reference(_) => {
                        continue;
                    }
                };
                if parent.type_name() == "DotnetRs.StubAttribute" {
                    let data = a.instantiation_data(self, &*support_res).map_err(|e| {
                        AssemblyLoadError::InvalidFormat(format!(
                            "failed to parse stub attribute data: {}",
                            e
                        ))
                    })?;
                    for n in data.named_args {
                        match n {
                            NamedArg::Field(name, FixedArg::String(Some(target)))
                                if name == "InPlaceOf" =>
                            {
                                let type_index =
                                    support_res.type_definition_index(index).ok_or_else(|| {
                                        AssemblyLoadError::InvalidFormat(
                                            "failed to find type definition index".to_string(),
                                        )
                                    })?;
                                let support_type_name = t.type_name();
                                self.stubs.insert(
                                    target.to_string(),
                                    TypeDescription::new(
                                        ResolutionS::new(support_res),
                                        t,
                                        type_index,
                                    ),
                                );
                                self.reverse_stubs
                                    .insert(support_type_name, target.to_string());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
