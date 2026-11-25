// Every test that starts child processes (CLI, host I/O, user functions, files), help pages and language corpora; no in-process MAT v7.3 users.
#[path = "../bench_cli.rs"]
mod bench_cli;
#[path = "../cli_contract.rs"]
mod cli_contract;
#[path = "../cli_streaming.rs"]
mod cli_streaming;
#[path = "../dicom_collection.rs"]
mod dicom_collection;
#[path = "../dicom_deflated.rs"]
mod dicom_deflated;
#[path = "../dicom_dict_custom.rs"]
mod dicom_dict_custom;
#[path = "../dicom_dictionary.rs"]
mod dicom_dictionary;
#[path = "../dicom_info.rs"]
mod dicom_info;
#[path = "../dicom_jpeg.rs"]
mod dicom_jpeg;
#[path = "../dicom_jpeg2000.rs"]
mod dicom_jpeg2000;
#[path = "../dicom_jpeg_lossless.rs"]
mod dicom_jpeg_lossless;
#[path = "../dicom_pixels.rs"]
mod dicom_pixels;
#[path = "../dicom_rle.rs"]
mod dicom_rle;
#[path = "../dicom_write.rs"]
mod dicom_write;
#[path = "../dicom_write_meta.rs"]
mod dicom_write_meta;
#[path = "../file_builtins.rs"]
mod file_builtins;
#[path = "../fn_envfiles.rs"]
mod fn_envfiles;
#[path = "../fn_files2.rs"]
mod fn_files2;
#[path = "../fn_lang.rs"]
mod fn_lang;
#[path = "../fn_lang2.rs"]
mod fn_lang2;
#[path = "../fn_lang3.rs"]
mod fn_lang3;
#[path = "../fn_misc.rs"]
mod fn_misc;
#[path = "../folder_kinds.rs"]
mod folder_kinds;
#[path = "../help_system.rs"]
mod help_system;
#[path = "../host_io.rs"]
mod host_io;
#[path = "../host_limits.rs"]
mod host_limits;
#[path = "../native_extensions.rs"]
mod native_extensions;
#[path = "../probe_mode.rs"]
mod probe_mode;
#[path = "../profiler.rs"]
mod profiler;
#[path = "../script_conflicts.rs"]
mod script_conflicts;
#[path = "../script_matrix_files.rs"]
mod script_matrix_files;
#[path = "../search_path.rs"]
mod search_path;
#[path = "../user_functions.rs"]
mod user_functions;
#[path = "../video_ffmpeg.rs"]
mod video_ffmpeg;
#[path = "../video_read.rs"]
mod video_read;
#[path = "../video_write.rs"]
mod video_write;
#[path = "../visdiff.rs"]
mod visdiff;
