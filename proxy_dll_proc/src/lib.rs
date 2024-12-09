use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, punctuated::Punctuated, Ident, Token};

/// helper function to strip comments from def files because the msvc_def crate does not handle them
fn strip_comments(input: &str) -> String {
    input
        .lines()
        .filter(|l| !l.trim_ascii_start().starts_with(";"))
        .collect::<Vec<&str>>()
        .join("\n")
}

struct Input {
    dlls: Punctuated<Ident, Token![,]>,
    entrypoint: Ident,
}

impl syn::parse::Parse for Input {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        syn::bracketed!(content in input);
        let dlls = content.parse_terminated(Ident::parse, Token![,])?;
        input.parse::<Token![,]>()?;
        let entrypoint = input.parse()?;
        Ok(Input { dlls, entrypoint })
    }
}

#[proc_macro]
pub fn proxy_dll(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as Input);

    let mut targets = vec![];
    let mut export_fns = vec![];
    let mut setup_forwards = vec![];
    let mut forward_counts = vec![];

    for (target_n, target) in parsed.dlls.iter().enumerate() {
        let dll = format!("{target}.dll");

        let file_string =
            std::fs::read_to_string(format!("{}/defs/{target}.def", env!("CARGO_MANIFEST_DIR")))
                .unwrap();
        let file = msvc_def::parse(&strip_comments(&file_string)).unwrap();

        targets.push(quote! {#dll});

        let mut count: usize = 0;
        for export in file.exports {
            let name = export.name;
            if name == "DllMain" {
                continue;
            }

            let name_ident = format_ident!("{name}");

            export_fns.push(quote! {
                #[no_mangle]
                pub unsafe extern "C" fn #name_ident() {
                    core::mem::transmute::<usize, unsafe extern "C" fn()>(FORWARDS[#target_n][#count])();
                }
            });

            let cstr_name = std::ffi::CString::new(name).unwrap();

            let name_or_ord = if export.noname {
                let ord = export.ordinal.unwrap();
                quote! {
                    #ord as *const u8
                }
            } else {
                quote! {
                    #cstr_name.as_ptr() as *const u8
                }
            };

            setup_forwards.push(quote! {
                FORWARDS[#target_n][#count] = GetProcAddress(TARGET_DLL_HANDLES[#target_n], #name_or_ord).unwrap_unchecked() as usize;
            });

            count += 1;
        }
        forward_counts.push(count);
    }

    let entrypoint = parsed.entrypoint;
    let target_count = targets.len();

    let output = quote! {
        unsafe extern "system" fn __call_entrypoint(_: usize) {
            #entrypoint();
        }
        #[cfg(windows)]
        mod __proxy_dll {
            const TARGETS: &[&str] = &[#(#targets,)*];
            static mut TARGET_DLL_HANDLES: &mut [HMODULE] = &mut [std::ptr::null_mut(); #target_count];
            static mut FORWARDS: &mut [&mut [usize]] = &mut [#(&mut [0; #forward_counts],)*];

            static mut INIT: bool = false;

            mod exports {
                use super::*;
                #(#export_fns)*
            }

            unsafe fn setup_forwards() {
                #(#setup_forwards)*
            }

            use ::proxy_dll::windows_sys::{
                Win32::{
                    Foundation::{FreeLibrary, BOOL, HMODULE, MAX_PATH},
                    System::{
                        LibraryLoader::{GetProcAddress, LoadLibraryA, LoadLibraryW},
                        SystemInformation::GetSystemDirectoryW,
                        SystemServices::{DLL_PROCESS_ATTACH, DLL_PROCESS_DETACH},
                        Threading::{ExitProcess, GetCurrentProcess, GetCurrentThread, QueueUserAPC},
                    },
                },
            };

            unsafe fn load_targets() {
                let Ok(dir) = ::proxy_dll::get_system_directory() else {
                    return;
                };

                for (i, target) in TARGETS.iter().enumerate() {
                    use std::os::windows::ffi::OsStrExt;
                    let mut dll_path = dir.join(target).into_os_string();
                    dll_path.push("\0");

                    TARGET_DLL_HANDLES[i] = LoadLibraryW(dll_path.encode_wide().collect::<Vec<u16>>().as_ptr());

                    if TARGET_DLL_HANDLES[i].is_null() {
                        ExitProcess(0);
                    }
                }
            }

            #[no_mangle]
            pub extern "system" fn DllMain(
                _hinst_dll: HMODULE,
                fdw_reason: u32,
                _lpv_reserved: *mut core::ffi::c_void,
            ) -> BOOL {
                match fdw_reason {
                    DLL_PROCESS_ATTACH => unsafe {
                        if !INIT {
                            INIT = true;
                            load_targets();
                            setup_forwards();
                            QueueUserAPC(Some(super::__call_entrypoint), GetCurrentThread(), 0);
                        }
                    },
                    DLL_PROCESS_DETACH => unsafe {
                        for handle in TARGET_DLL_HANDLES.iter() {
                            FreeLibrary(*handle);
                        }
                    },
                    _ => {}
                }
                true.into()
            }
        }
    };

    output.into()
}
