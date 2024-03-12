//! Receive command from the cli and call the respective function for that command.

mod compress;
mod decompress;
mod list;

use std::{
    ops::ControlFlow,
    path::PathBuf,
    sync::{
        mpsc::{channel, Sender},
        Arc, Condvar, Mutex,
    },
};

use rayon::prelude::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};
use utils::colors;

use crate::{
    accessible::is_running_in_accessible_mode,
    check,
    cli::Subcommand,
    commands::{compress::compress_files, decompress::decompress_file, list::list_archive_contents},
    error::{Error, FinalError},
    extension::{self, parse_format},
    list::ListOptions,
    utils::{
        self,
        message::{MessageLevel, PrintMessage},
        to_utf, EscapedPathDisplay, FileVisibilityPolicy,
    },
    CliArgs, QuestionPolicy,
};

/// Warn the user that (de)compressing this .zip archive might freeze their system.
fn warn_user_about_loading_zip_in_memory(log_sender: Sender<PrintMessage>) {
    const ZIP_IN_MEMORY_LIMITATION_WARNING: &str = "\n\
        \tThe format '.zip' is limited and cannot be (de)compressed using encoding streams.\n\
        \tWhen using '.zip' with other formats, (de)compression must be done in-memory\n\
        \tCareful, you might run out of RAM if the archive is too large!";

    log_sender
        .send(PrintMessage {
            contents: ZIP_IN_MEMORY_LIMITATION_WARNING.to_string(),
            accessible: true,
            level: MessageLevel::Warning,
        })
        .unwrap();
}

/// Warn the user that (de)compressing this .7z archive might freeze their system.
fn warn_user_about_loading_sevenz_in_memory(log_sender: Sender<PrintMessage>) {
    const SEVENZ_IN_MEMORY_LIMITATION_WARNING: &str = "\n\
        \tThe format '.7z' is limited and cannot be (de)compressed using encoding streams.\n\
        \tWhen using '.7z' with other formats, (de)compression must be done in-memory\n\
        \tCareful, you might run out of RAM if the archive is too large!";

    log_sender
        .send(PrintMessage {
            contents: SEVENZ_IN_MEMORY_LIMITATION_WARNING.to_string(),
            accessible: true,
            level: MessageLevel::Warning,
        })
        .unwrap();
}

/// This function checks what command needs to be run and performs A LOT of ahead-of-time checks
/// to assume everything is OK.
///
/// There are a lot of custom errors to give enough error description and explanation.
pub fn run(
    args: CliArgs,
    question_policy: QuestionPolicy,
    file_visibility_policy: FileVisibilityPolicy,
) -> crate::Result<()> {
    let (log_sender, log_receiver) = channel::<PrintMessage>();

    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair2 = Arc::clone(&pair);

    // Log received messages until all senders are dropped
    rayon::spawn(move || {
        use utils::colors::{ORANGE, RESET, YELLOW};

        const BUFFER_SIZE: usize = 10;
        let mut buffer = Vec::<String>::with_capacity(BUFFER_SIZE);

        // TODO: Move this out to utils
        fn map_message(msg: &PrintMessage) -> Option<String> {
            match msg.level {
                MessageLevel::Info => {
                    if msg.accessible {
                        if is_running_in_accessible_mode() {
                            Some(format!("{}Info:{} {}", *YELLOW, *RESET, msg.contents))
                        } else {
                            Some(format!("{}[INFO]{} {}", *YELLOW, *RESET, msg.contents))
                        }
                    } else if !is_running_in_accessible_mode() {
                        Some(format!("{}[INFO]{} {}", *YELLOW, *RESET, msg.contents))
                    } else {
                        None
                    }
                }
                MessageLevel::Warning => {
                    if is_running_in_accessible_mode() {
                        Some(format!("{}Warning:{} ", *ORANGE, *RESET))
                    } else {
                        Some(format!("{}[WARNING]{} ", *ORANGE, *RESET))
                    }
                }
            }
        }

        loop {
            let msg = log_receiver.recv();

            // Senders are still active
            if let Ok(msg) = msg {
                // Print messages if buffer is full otherwise append to it
                if buffer.len() == BUFFER_SIZE {
                    let mut tmp = buffer.join("\n");

                    if let Some(msg) = map_message(&msg) {
                        tmp.push_str(&msg);
                    }

                    // TODO: Send this to stderr
                    println!("{}", tmp);
                    buffer.clear();
                } else if let Some(msg) = map_message(&msg) {
                    buffer.push(msg);
                }
            } else {
                // All senders have been dropped
                // TODO: Send this to stderr
                println!("{}", buffer.join("\n"));

                // Wake up the main thread
                let (lock, cvar) = &*pair2;
                let mut flushed = lock.lock().unwrap();
                *flushed = true;
                cvar.notify_one();
                break;
            }
        }
    });

    match args.cmd {
        Subcommand::Compress {
            files,
            output: output_path,
            level,
            fast,
            slow,
        } => {
            // After cleaning, if there are no input files left, exit
            if files.is_empty() {
                return Err(FinalError::with_title("No files to compress").into());
            }

            // Formats from path extension, like "file.tar.gz.xz" -> vec![Tar, Gzip, Lzma]
            let (formats_from_flag, formats) = match args.format {
                Some(formats) => {
                    let parsed_formats = parse_format(&formats)?;
                    (Some(formats), parsed_formats)
                }
                None => (None, extension::extensions_from_path(&output_path, log_sender.clone())),
            };

            check::check_invalid_compression_with_non_archive_format(
                &formats,
                &output_path,
                &files,
                formats_from_flag.as_ref(),
            )?;
            check::check_archive_formats_position(&formats, &output_path)?;

            let output_file = match utils::ask_to_create_file(&output_path, question_policy)? {
                Some(writer) => writer,
                None => return Ok(()),
            };

            let level = if fast {
                Some(1) // Lowest level of compression
            } else if slow {
                Some(i16::MAX) // Highest level of compression
            } else {
                level
            };

            let compress_result = compress_files(
                files,
                formats,
                output_file,
                &output_path,
                args.quiet,
                question_policy,
                file_visibility_policy,
                level,
                log_sender.clone(),
            );

            if let Ok(true) = compress_result {
                // this is only printed once, so it doesn't result in much text. On the other hand,
                // having a final status message is important especially in an accessibility context
                // as screen readers may not read a commands exit code, making it hard to reason
                // about whether the command succeeded without such a message
                log_sender
                    .send(PrintMessage {
                        contents: format!("Successfully compressed '{}'.", to_utf(&output_path)),
                        accessible: true,
                        level: MessageLevel::Info,
                    })
                    .unwrap();
            } else {
                // If Ok(false) or Err() occurred, delete incomplete file at `output_path`
                //
                // if deleting fails, print an extra alert message pointing
                // out that we left a possibly CORRUPTED file at `output_path`
                if utils::remove_file_or_dir(&output_path).is_err() {
                    eprintln!("{red}FATAL ERROR:\n", red = *colors::RED);
                    eprintln!(
                        "  Ouch failed to delete the file '{}'.",
                        EscapedPathDisplay::new(&output_path)
                    );
                    eprintln!("  Please delete it manually.");
                    eprintln!("  This file is corrupted if compression didn't finished.");

                    if compress_result.is_err() {
                        eprintln!("  Compression failed for reasons below.");
                    }
                }
            }

            compress_result?;
        }
        Subcommand::Decompress { files, output_dir } => {
            let mut output_paths = vec![];
            let mut formats = vec![];

            if let Some(format) = args.format {
                let format = parse_format(&format)?;
                for path in files.iter() {
                    let file_name = path.file_name().ok_or_else(|| Error::NotFound {
                        error_title: format!("{} does not have a file name", EscapedPathDisplay::new(path)),
                    })?;
                    output_paths.push(file_name.as_ref());
                    formats.push(format.clone());
                }
            } else {
                for path in files.iter() {
                    let (pathbase, mut file_formats) =
                        extension::separate_known_extensions_from_name(path, log_sender.clone());

                    if let ControlFlow::Break(_) =
                        check::check_mime_type(path, &mut file_formats, question_policy, log_sender.clone())?
                    {
                        return Ok(());
                    }

                    output_paths.push(pathbase);
                    formats.push(file_formats);
                }
            }

            check::check_missing_formats_when_decompressing(&files, &formats)?;

            // The directory that will contain the output files
            // We default to the current directory if the user didn't specify an output directory with --dir
            let output_dir = if let Some(dir) = output_dir {
                utils::create_dir_if_non_existent(&dir, log_sender.clone())?;
                dir
            } else {
                PathBuf::from(".")
            };

            files
                .par_iter()
                .zip(formats)
                .zip(output_paths)
                .try_for_each(|((input_path, formats), file_name)| {
                    let output_file_path = output_dir.join(file_name); // Path used by single file format archives
                    decompress_file(
                        input_path,
                        formats,
                        &output_dir,
                        output_file_path,
                        question_policy,
                        args.quiet,
                        log_sender.clone(),
                    )
                })?;
        }
        Subcommand::List { archives: files, tree } => {
            let mut formats = vec![];

            if let Some(format) = args.format {
                let format = parse_format(&format)?;
                for _ in 0..files.len() {
                    formats.push(format.clone());
                }
            } else {
                for path in files.iter() {
                    let mut file_formats = extension::extensions_from_path(path, log_sender.clone());

                    if let ControlFlow::Break(_) =
                        check::check_mime_type(path, &mut file_formats, question_policy, log_sender.clone())?
                    {
                        return Ok(());
                    }

                    formats.push(file_formats);
                }
            }

            // Ensure we were not told to list the content of a non-archive compressed file
            check::check_for_non_archive_formats(&files, &formats)?;

            let list_options = ListOptions { tree };

            for (i, (archive_path, formats)) in files.iter().zip(formats).enumerate() {
                if i > 0 {
                    println!();
                }
                let formats = extension::flatten_compression_formats(&formats);
                list_archive_contents(archive_path, formats, list_options, question_policy, log_sender.clone())?;
            }
        }
    }

    // Drop our sender so when all threads are done, no clones are left
    drop(log_sender);

    // Prevent the main thread from exiting until the background thread handling the
    // logging has set `flushed` to true.
    let (lock, cvar) = &*pair;
    let guard = lock.lock().unwrap();
    let _flushed = cvar.wait(guard).unwrap();

    Ok(())
}
