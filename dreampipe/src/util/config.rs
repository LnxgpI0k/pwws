#![allow(unused)]

use crate::error::CompositorError;
use crate::error::CompositorResult;
use microxdg::Xdg;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::path::Path as FsPath;
use std::path::PathBuf;
use std::str::FromStr;
use tracing::warn;

pub struct CompositorConfig;

impl CompositorConfig {
   pub fn config_path() -> Option<PathBuf> {
      let xdg = Xdg::new().ok();
      let config_dir = xdg.map(|xdg| xdg.config().ok()).flatten();
      config_dir.map(|dir| dir.to_path_buf().join("pwws").join("pwws.kv"))
   }

   pub fn offset_key(display_name: &str) -> String {
      format!["{display_name}.offset"]
   }
}

/// Format is: key = value. Comments are with '#' or just write anything. Errors
/// won't be raised on "extra" data, equals-sign present or not. Outer whitespace
/// is stripped.
// Simple K-V store
#[derive(Default)]
pub struct Config {
   data: HashMap<String, String>,
}

impl Config {
   /// Load from file
   pub fn from_str(s: &str) -> Self {
      let mut cfg = Self { data: HashMap::new() };
      {
         let data = &mut cfg.data;
         for (i, line) in s.lines().enumerate() {
            let i = i + 1;
            let line = line.chars().take_while(|ch| *ch != '#').collect::<String>();
            let line = line.trim();
            if line != "" {
               let (k, v) = if let Some(v) = line.split_once("=") {
                  v
               } else {
                  // don't stress it
                  warn!["Missing '=' on line {i}"];
                  continue;
               };
               data.insert(k.trim().to_string(), v.trim().to_string());
            }
         }
      }
      cfg
   }

   /// Load from file
   pub fn new(path: impl AsRef<FsPath>) -> CompositorResult<Self> {
      let mut buf = String::new();
      {
         let mut f = File::open(path).map_err(|e| CompositorError::ConfigOpen(e))?;
         f.read_to_string(&mut buf).map_err(|e| CompositorError::ConfigRead(e))?;
      }
      Ok(Self::from_str(&buf))
   }

   /// Get a config value
   pub fn get<N: FromStr>(&self, k: &str) -> Option<N>
   where
      <N as FromStr>::Err: std::fmt::Display {
      if let Some(v) = self.data.get(k) {
         match N::from_str(v) {
            Ok(v) => Some(v),
            Err(e) => {
               tracing::error![
                  "Failed to parse '{v}' as {}: {e}",
                  std::any::type_name::<N>()
               ];
               None
            },
         }
      } else {
         None
      }
   }
}
