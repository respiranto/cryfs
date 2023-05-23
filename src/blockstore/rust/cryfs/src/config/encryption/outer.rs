use anyhow::{ensure, Context, Result};
use binrw::{binrw, until_eof, BinRead, BinWrite, NullString};
use std::io::{Cursor, Read, Seek, Write};

use cryfs_utils::crypto::{
    kdf::KDFParameters,
    symmetric::{Cipher, CipherDef, EncryptionKey},
};

use super::inner::InnerConfig;

const HEADER: &str = "cryfs.config;1;scrypt";

pub type OuterCipher = cryfs_utils::crypto::symmetric::Aes256Gcm;

#[binrw]
#[brw(little)]
struct OuterConfigLayout {
    header: NullString,

    kdf_parameters_num_bytes: u64,
    #[br(count = kdf_parameters_num_bytes)]
    kdf_parameters_serialized: Vec<u8>,

    #[br(parse_with = until_eof)]
    encrypted_inner_config: Vec<u8>,
}

/// Wraps an [InnerConfig] instance and encrypts it, then prepends the KDF parameters that were used
/// and a fixed header.
///
/// Common usage patterns are:
/// * When loading a cryfs config file, call first [OuterConfig::deserialize] to get an [OuterConfig] instance,
///   then call [OuterConfig::decrypt] to get the contained [InnerConfig] object.
/// * When storing a cryfs config file, call [OuterConfig::encrypt] to get an [OuterConfig] instance,
///   then call [OuterConfig::serialize] to get the serialized representation of the [OuterConfig] instance.
pub struct OuterConfig {
    kdf_parameters_serialized: Vec<u8>,
    encrypted_inner_config: Vec<u8>,
}

impl OuterConfig {
    pub fn encrypt(
        config: InnerConfig,
        kdf_parameters: impl KDFParameters,
        outer_encryption_key: EncryptionKey,
    ) -> Result<OuterConfig> {
        let mut serialized_inner_config = vec![];
        config
            .serialize(&mut Cursor::new(&mut serialized_inner_config))
            .context("Trying to serialize InnerConfig")?;
        let cipher = OuterCipher::new(outer_encryption_key)
            .context("Trying to initialize OuterCipher instance")?;
        let encrypted_inner_config = cipher
            .encrypt(serialized_inner_config.into())
            .context("Trying to Cipher::encrypt OuterConfig")?;
        Ok(Self {
            kdf_parameters_serialized: kdf_parameters.serialize(),
            encrypted_inner_config: encrypted_inner_config.into_vec(),
        })
    }

    pub fn decrypt(self, outer_encryption_key: EncryptionKey) -> Result<InnerConfig> {
        let cipher = OuterCipher::new(outer_encryption_key)
            .context("Trying to initialize OuterCipher instance")?;
        let plaintext = cipher
            .decrypt(self.encrypted_inner_config.into())
            .context("Trying to Cipher::decrypt OuterConfig")?;
        let inner_config = InnerConfig::deserialize(&mut Cursor::new(plaintext))
            .context("Trying to deserialize InnerConfig")?;
        Ok(inner_config)
    }

    pub fn deserialize(source: &mut (impl Read + Seek)) -> Result<Self> {
        let layout = OuterConfigLayout::read(source)?;
        let read_header: String = layout
            .header
            .try_into()
            .context("Header is not valid UTF-8")?;
        ensure!(
            read_header == HEADER,
            "Invalid header in outer config. Expected '{HEADER}', got '{read_header}'",
        );
        assert_eq!(
            layout.kdf_parameters_num_bytes,
            layout.kdf_parameters_serialized.len() as u64
        );
        Ok(Self {
            kdf_parameters_serialized: layout.kdf_parameters_serialized,
            encrypted_inner_config: layout.encrypted_inner_config,
        })
    }

    pub fn serialize(self, dest: &mut (impl Write + Seek)) -> Result<()> {
        let layout = OuterConfigLayout {
            header: HEADER.into(),
            kdf_parameters_num_bytes: self.kdf_parameters_serialized.len() as u64,
            kdf_parameters_serialized: self.kdf_parameters_serialized,
            encrypted_inner_config: self.encrypted_inner_config,
        };
        layout.write(dest)?;
        Ok(())
    }

    pub fn kdf_parameters(&self) -> &[u8] {
        &self.kdf_parameters_serialized
    }
}
