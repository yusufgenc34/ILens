use serde::Serialize;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidPe,
    NotManaged,
    InvalidClr,
    CorruptMetadata,
    UnsupportedMetadata,
    UnsupportedNative,
    InvalidMethodBody,
    DecompilerLimitation,
    SizeLimit,
    Cancelled,
    InvalidEdit,
    UnsupportedEdit,
    ExportError,
}
#[derive(Debug, Clone, Serialize)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    pub detail: String,
}
impl Error {
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        let message = match code {
            ErrorCode::InvalidPe => "This file is not a valid Windows PE assembly.",
            ErrorCode::NotManaged => "This file contains no CLR metadata. NativeAOT and native executables are not supported.",
            ErrorCode::InvalidClr => "The CLR header is damaged or incomplete.",
            ErrorCode::CorruptMetadata => "The assembly metadata is damaged or contains invalid references.",
            ErrorCode::UnsupportedMetadata => "This metadata format is not supported yet.",
            ErrorCode::UnsupportedNative => "This assembly contains native code. Only IL-only assemblies are supported.",
            ErrorCode::InvalidMethodBody => "This method has invalid or truncated CIL.",
            ErrorCode::DecompilerLimitation => "This method requires a decompilation feature that is not implemented yet. The IL view is available.",
            ErrorCode::SizeLimit => "This input exceeds the browser analysis limits.",
            ErrorCode::Cancelled => "The operation was cancelled.",
            ErrorCode::InvalidEdit => "The edited IL did not pass validation.",
            ErrorCode::UnsupportedEdit => "This method or assembly is outside the supported editing scope.",
            ErrorCode::ExportError => "The export could not be completed.",
        }.to_owned();
        Self {
            code,
            message,
            detail: detail.into(),
        }
    }
    pub fn metadata(detail: impl Into<String>) -> Self {
        Self::new(ErrorCode::CorruptMetadata, detail)
    }
    pub fn cil(detail: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidMethodBody, detail)
    }
    pub fn limit(detail: impl Into<String>) -> Self {
        Self::new(ErrorCode::SizeLimit, detail)
    }
    pub fn limitation(detail: impl Into<String>) -> Self {
        Self::new(ErrorCode::DecompilerLimitation, detail)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.message, self.detail)
    }
}
impl std::error::Error for Error {}
impl From<clrmeta::Error> for Error {
    fn from(e: clrmeta::Error) -> Self {
        Self::metadata(e.to_string())
    }
}
