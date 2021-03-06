use std::fmt::Debug;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use futures::Stream;

#[cfg(feature = "azure")]
use azure_core::errors::AzureError;

#[cfg(feature = "s3")]
use rusoto_core::RusotoError;

#[cfg(feature = "azure")]
pub mod azure;
pub mod file;
#[cfg(feature = "s3")]
pub mod s3;

#[derive(thiserror::Error, Debug)]
pub enum UriError {
    #[error("Invalid URI scheme: {0}")]
    InvalidScheme(String),
    #[error("Expected local path URI, found: {0}")]
    ExpectedSLocalPathUri(String),

    #[cfg(feature = "s3")]
    #[error("Object URI missing bucket")]
    MissingObjectBucket,
    #[cfg(feature = "s3")]
    #[error("Object URI missing key")]
    MissingObjectKey,
    #[cfg(feature = "s3")]
    #[error("Expected S3 URI, found: {0}")]
    ExpectedS3Uri(String),

    #[cfg(feature = "azure")]
    #[error("Expected Azure URI, found: {0}")]
    ExpectedAzureUri(String),
    #[cfg(feature = "azure")]
    #[error("Object URI missing filesystem")]
    MissingObjectFileSystem,
    #[cfg(feature = "azure")]
    #[error("Object URI missing account name and path")]
    MissingObjectAccountAndPath,
    #[cfg(feature = "azure")]
    #[error("Object URI missing account name")]
    MissingObjectAccountName,
    #[cfg(feature = "azure")]
    #[error("Object URI missing path")]
    MissingObjectPath,
}

#[derive(Debug)]
pub enum Uri<'a> {
    LocalPath(&'a str),
    #[cfg(feature = "s3")]
    S3Object(s3::S3Object<'a>),
    #[cfg(feature = "azure")]
    ADLSGen2Object(azure::ADLSGen2Object<'a>),
}

impl<'a> Uri<'a> {
    #[cfg(feature = "s3")]
    pub fn into_s3object(self) -> Result<s3::S3Object<'a>, UriError> {
        match self {
            Uri::S3Object(x) => Ok(x),
            #[cfg(feature = "azure")]
            Uri::ADLSGen2Object(x) => Err(UriError::ExpectedS3Uri(x.to_string())),
            Uri::LocalPath(x) => Err(UriError::ExpectedS3Uri(x.to_string())),
        }
    }

    #[cfg(feature = "azure")]
    pub fn into_adlsgen2_object(self) -> Result<azure::ADLSGen2Object<'a>, UriError> {
        match self {
            Uri::ADLSGen2Object(x) => Ok(x),
            #[cfg(feature = "s3")]
            Uri::S3Object(x) => Err(UriError::ExpectedAzureUri(x.to_string())),
            Uri::LocalPath(x) => Err(UriError::ExpectedAzureUri(x.to_string())),
        }
    }

    pub fn into_localpath(self) -> Result<&'a str, UriError> {
        match self {
            Uri::LocalPath(x) => Ok(x),
            #[cfg(feature = "s3")]
            Uri::S3Object(x) => Err(UriError::ExpectedSLocalPathUri(format!("{}", x))),
            #[cfg(feature = "azure")]
            Uri::ADLSGen2Object(x) => Err(UriError::ExpectedSLocalPathUri(format!("{}", x))),
        }
    }
}

pub fn parse_uri<'a>(path: &'a str) -> Result<Uri<'a>, UriError> {
    let parts: Vec<&'a str> = path.split("://").collect();

    if parts.len() == 1 {
        return Ok(Uri::LocalPath(parts[0]));
    }

    match parts[0] {
        "s3" => {
            cfg_if::cfg_if! {
                if #[cfg(feature = "s3")] {
                    let mut path_parts = parts[1].splitn(2, '/');
                    let bucket = match path_parts.next() {
                        Some(x) => x,
                        None => {
                            return Err(UriError::MissingObjectBucket);
                        }
                    };
                    let key = match path_parts.next() {
                        Some(x) => x,
                        None => {
                            return Err(UriError::MissingObjectKey);
                        }
                    };

                    Ok(Uri::S3Object(s3::S3Object { bucket, key }))
                } else {
                    Err(UriError::InvalidScheme(String::from(parts[0])))
                }
            }
        }
        "file" => Ok(Uri::LocalPath(parts[1])),
        "abfss" => {
            cfg_if::cfg_if! {
                if #[cfg(feature = "azure")] {
                    let mut parts = parts[1].splitn(2, '@');
                    let file_system = parts.next().ok_or(UriError::MissingObjectFileSystem)?;
                    let mut parts = parts.next().map(|x| x.splitn(2, '.')).ok_or(UriError::MissingObjectAccountAndPath)?;
                    let account_name = parts.next().ok_or(UriError::MissingObjectAccountName)?;
                    let mut paths = parts.next().map(|x| x.splitn(2, '/')).ok_or(UriError::MissingObjectPath)?;
                    let path = paths.nth(1).ok_or(UriError::MissingObjectPath)?;
                    Ok(Uri::ADLSGen2Object(azure::ADLSGen2Object { account_name, file_system, path }))
                } else {
                    Err(UriError::InvalidScheme(String::from(parts[0])))
                }
            }
        }
        _ => Err(UriError::InvalidScheme(String::from(parts[0]))),
    }
}

#[derive(thiserror::Error, Debug)]
pub enum StorageError {
    #[error("Object not found")]
    NotFound,
    #[error("Failed to read local object content: {source}")]
    IO { source: std::io::Error },

    #[cfg(feature = "s3")]
    #[error("Failed to read S3 object content: {source}")]
    S3Get {
        source: RusotoError<rusoto_s3::GetObjectError>,
    },
    #[cfg(feature = "s3")]
    #[error("Failed to read S3 object metadata: {source}")]
    S3Head {
        source: RusotoError<rusoto_s3::HeadObjectError>,
    },
    #[cfg(feature = "s3")]
    #[error("Failed to list S3 objects: {source}")]
    S3List {
        source: RusotoError<rusoto_s3::ListObjectsV2Error>,
    },
    #[cfg(feature = "s3")]
    #[error("S3 Object missing body content: {0}")]
    S3MissingObjectBody(String),

    #[cfg(feature = "azure")]
    #[error("Error interacting with Azure: {source}")]
    Azure { source: AzureError },

    #[error("Invalid object URI")]
    Uri {
        #[from]
        source: UriError,
    },
}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => StorageError::NotFound,
            _ => StorageError::IO { source: error },
        }
    }
}

#[cfg(feature = "s3")]
impl From<RusotoError<rusoto_s3::GetObjectError>> for StorageError {
    fn from(error: RusotoError<rusoto_s3::GetObjectError>) -> Self {
        match error {
            RusotoError::Service(rusoto_s3::GetObjectError::NoSuchKey(_)) => StorageError::NotFound,
            _ => StorageError::S3Get { source: error },
        }
    }
}

#[cfg(feature = "s3")]
impl From<RusotoError<rusoto_s3::HeadObjectError>> for StorageError {
    fn from(error: RusotoError<rusoto_s3::HeadObjectError>) -> Self {
        match error {
            RusotoError::Service(rusoto_s3::HeadObjectError::NoSuchKey(_)) => {
                StorageError::NotFound
            }
            _ => StorageError::S3Head { source: error },
        }
    }
}

#[cfg(feature = "s3")]
impl From<RusotoError<rusoto_s3::ListObjectsV2Error>> for StorageError {
    fn from(error: RusotoError<rusoto_s3::ListObjectsV2Error>) -> Self {
        match error {
            RusotoError::Service(rusoto_s3::ListObjectsV2Error::NoSuchBucket(_)) => {
                StorageError::NotFound
            }
            _ => StorageError::S3List { source: error },
        }
    }
}

#[cfg(feature = "azure")]
impl From<AzureError> for StorageError {
    fn from(error: AzureError) -> Self {
        match error {
            AzureError::UnexpectedHTTPResult(e) if e.status_code().as_u16() == 404 => {
                StorageError::NotFound
            }
            _ => StorageError::Azure { source: error },
        }
    }
}

pub struct ObjectMeta {
    pub path: String,
    // The timestamp of a commit comes from the remote storage `lastModifiedTime`, and can be
    // adjusted for clock skew.
    pub modified: DateTime<Utc>,
}

#[async_trait::async_trait]
pub trait StorageBackend: Send + Sync + Debug {
    async fn head_obj(&self, path: &str) -> Result<ObjectMeta, StorageError>;
    async fn get_obj(&self, path: &str) -> Result<Vec<u8>, StorageError>;
    async fn list_objs<'a>(
        &'a self,
        path: &'a str,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ObjectMeta, StorageError>> + 'a>>, StorageError>;
}

pub fn get_backend_for_uri(uri: &str) -> Result<Box<dyn StorageBackend>, UriError> {
    match parse_uri(uri)? {
        Uri::LocalPath(_) => Ok(Box::new(file::FileStorageBackend::new())),
        #[cfg(feature = "s3")]
        Uri::S3Object(_) => Ok(Box::new(s3::S3StorageBackend::new())),
        #[cfg(feature = "azure")]
        Uri::ADLSGen2Object(_) => Ok(Box::new(azure::ADLSGen2Backend::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_uri_local_file() {
        let uri = parse_uri("foo/bar").unwrap();
        assert_eq!(uri.into_localpath().unwrap(), "foo/bar");

        let uri2 = parse_uri("file:///foo/bar").unwrap();
        assert_eq!(uri2.into_localpath().unwrap(), "/foo/bar");
    }

    #[cfg(feature = "s3")]
    #[test]
    fn test_parse_s3_object_uri() {
        let uri = parse_uri("s3://foo/bar").unwrap();
        assert_eq!(
            uri.into_s3object().unwrap(),
            s3::S3Object {
                bucket: "foo",
                key: "bar",
            }
        );
    }

    #[cfg(feature = "azure")]
    #[test]
    fn test_parse_azure_object_uri() {
        let uri = parse_uri("abfss://fs@sa.dfs.core.windows.net/foo").unwrap();
        assert_eq!(
            uri.into_adlsgen2_object().unwrap(),
            azure::ADLSGen2Object {
                account_name: "sa",
                file_system: "fs",
                path: "foo",
            }
        );
    }
}
