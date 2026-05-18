use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::{
    env,
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
};

use crate::AbsintheError;

pub fn bin_serialize<S>(x: &[u8], s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    s.serialize_bytes(x)
}

pub fn bin_deserialize_mac<'de, D>(d: D) -> Result<[u8; 6], D::Error>
where
    D: Deserializer<'de>,
{
    bin_deserialize(d).map(|i| i.try_into().unwrap())
}

pub fn bin_deserialize<'de, D>(d: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    use core::fmt;

    struct DataVisitor;

    impl<'de> de::Visitor<'de> for DataVisitor {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a byte array")
        }

        fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v.to_vec())
        }

        fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v)
        }
    }

    d.deserialize_byte_buf(DataVisitor)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareConfig {
    pub product_name: String,
    #[serde(serialize_with = "bin_serialize", deserialize_with = "bin_deserialize_mac")]
    pub io_mac_address: [u8; 6],
    pub platform_serial_number: String,
    pub platform_uuid: String,
    pub root_disk_uuid: String,
    pub board_id: String,
    pub os_build_num: String,
    #[serde(serialize_with = "bin_serialize", deserialize_with = "bin_deserialize")]
    pub platform_serial_number_enc: Vec<u8>,
    #[serde(serialize_with = "bin_serialize", deserialize_with = "bin_deserialize")]
    pub platform_uuid_enc: Vec<u8>,
    #[serde(serialize_with = "bin_serialize", deserialize_with = "bin_deserialize")]
    pub root_disk_uuid_enc: Vec<u8>,
    #[serde(serialize_with = "bin_serialize", deserialize_with = "bin_deserialize")]
    pub rom: Vec<u8>,
    #[serde(serialize_with = "bin_serialize", deserialize_with = "bin_deserialize")]
    pub rom_enc: Vec<u8>,
    pub mlb: String,
    #[serde(serialize_with = "bin_serialize", deserialize_with = "bin_deserialize")]
    pub mlb_enc: Vec<u8>,
}

impl HardwareConfig {
    pub fn from_validation_data(_data: &[u8]) -> Result<HardwareConfig, AbsintheError> {
        panic!("Not supported with hosted validation helper");
    }
}

#[derive(Serialize)]
struct InitialPayload {
    hardware_config: HardwareConfig,
    cert_data: Vec<u8>,
}

#[derive(Deserialize)]
struct SessionInfoPayload {
    session_info: Vec<u8>,
}

#[derive(Serialize)]
struct AppleSessionInfoPayload {
    session_info: Vec<u8>,
}

#[derive(Deserialize)]
struct ResultPayload {
    result: Vec<u8>,
}

pub struct ValidationCtx {
    child: Child,
}

impl ValidationCtx {
    pub fn new(
        cert_chain: &[u8],
        out_request_bytes: &mut Vec<u8>,
        hw_config: &HardwareConfig,
    ) -> Result<ValidationCtx, AbsintheError> {
        let binary =
            env::var("MACOS_VALIDATION_DATA_BIN").unwrap_or_else(|_| "macos-validation-data".into());
        let mut child = Command::new(binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("failed to start macos-validation-data");

        let initial = InitialPayload {
            hardware_config: hw_config.clone(),
            cert_data: cert_chain.to_vec(),
        };
        write_json_line(&mut child, &initial);

        let response: SessionInfoPayload = read_json_line(&mut child);
        out_request_bytes.extend(response.session_info);

        Ok(Self { child })
    }

    pub fn key_establishment(&mut self, response: &[u8]) -> Result<(), AbsintheError> {
        write_json_line(
            &mut self.child,
            &AppleSessionInfoPayload {
                session_info: response.to_vec(),
            },
        );
        Ok(())
    }

    pub fn sign(&mut self) -> Result<Vec<u8>, AbsintheError> {
        let response: ResultPayload = read_json_line(&mut self.child);
        Ok(response.result)
    }
}

fn write_json_line<T: Serialize>(child: &mut Child, value: &T) {
    let stdin = child.stdin.as_mut().expect("missing helper stdin");
    serde_json::to_writer(&mut *stdin, value).expect("failed to write helper payload");
    stdin.write_all(b"\n").expect("failed to write newline");
    stdin.flush().expect("failed to flush helper stdin");
}

fn read_json_line<T: for<'de> Deserialize<'de>>(child: &mut Child) -> T {
    let stdout = child.stdout.as_mut().expect("missing helper stdout");
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("failed to read helper response");
    serde_json::from_str(&line).expect("failed to parse helper response")
}
