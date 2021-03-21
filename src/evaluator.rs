
use colored::Colorize;

use crate::{cli::{Command, CommandKind}, error, extension::CompressionFormat, file::File};
use crate::decompressors::Decompressor;
use crate::decompressors::TarDecompressor;
use crate::decompressors::ZipDecompressor;

pub struct Evaluator {   
    command: Command,
    // verbosity: Verbosity
}

impl Evaluator {
    // todo: remove this?
    pub fn new(command: Command) -> Self {
        Self {
            command
        }
    }

    fn get_decompressor(&self, file: &File) -> error::OuchResult<Box<dyn Decompressor>> {
        if file.extension.is_none() {
            // This block *should* be unreachable
            eprintln!("{}: reached Evaluator::get_decompressor without known extension.", "internal error".red());
            return Err(error::Error::InvalidInput);
        }
        let extension = file.extension.clone().unwrap();
        let decompressor: Box<dyn Decompressor> = match extension.second_ext {
            CompressionFormat::Tar => { 
                Box::new(TarDecompressor{})
            },
            CompressionFormat::Zip => { 
                Box::new(ZipDecompressor{})
            }
            _ => { 
                todo!()
            }
        };


        Ok(decompressor)
    }

    fn decompress_file(&self, file: &File) -> error::OuchResult<()> {
        let output_file = &self.command.output;
        let decompressor = self.get_decompressor(file)?;
        let files_unpacked = decompressor.decompress(file, output_file)?;

        // TODO: decompress the first extension if it exists

        Ok(())
    }

    pub fn evaluate(&mut self) -> error::OuchResult<()> {
        match &self.command.kind {
            CommandKind::Compression(files_to_compress) => {
                for _file in files_to_compress {
                    todo!();
                }
            }
            CommandKind::Decompression(files_to_decompress) => {
                for file in files_to_decompress {
                    self.decompress_file(file)?;
                }
            }
        }
        Ok(())
    }
}