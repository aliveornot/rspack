use std::{fmt::Write, hash::Hash, path::Path};

use heck::{ToKebabCase, ToLowerCamelCase};
use indexmap::IndexMap;
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use rspack_core::{Compilation, ModuleDependency, OutputOptions, PathData, RuntimeGlobals};
use rspack_error::{internal_error, Result};
use rspack_hash::{HashDigest, HashFunction, HashSalt, RspackHash};
use swc_core::css::modules::CssClassName;
use swc_core::ecma::atoms::JsWord;

use crate::plugin::{LocalIdentName, LocalIdentNameRenderOptions, LocalsConvention};

pub const AUTO_PUBLIC_PATH_PLACEHOLDER: &str = "__RSPACK_PLUGIN_CSS_AUTO_PUBLIC_PATH__";
pub static AUTO_PUBLIC_PATH_PLACEHOLDER_REGEX: Lazy<Regex> =
  Lazy::new(|| Regex::new(AUTO_PUBLIC_PATH_PLACEHOLDER).expect("Invalid regexp"));

pub struct ModulesTransformConfig<'a> {
  filename: &'a Path,
  local_name_ident: &'a LocalIdentName,
  hash_function: &'a HashFunction,
  hash_digest: &'a HashDigest,
  hash_digest_length: usize,
  hash_salt: &'a HashSalt,
}

impl<'a> ModulesTransformConfig<'a> {
  pub fn new(
    filename: &'a Path,
    local_name_ident: &'a LocalIdentName,
    output: &'a OutputOptions,
  ) -> Self {
    Self {
      filename,
      local_name_ident,
      hash_function: &output.hash_function,
      hash_digest: &output.hash_digest,
      hash_digest_length: output.hash_digest_length,
      hash_salt: &output.hash_salt,
    }
  }
}

impl swc_core::css::modules::TransformConfig for ModulesTransformConfig<'_> {
  fn new_name_for(&self, local: &JsWord) -> JsWord {
    let hash = {
      let mut hasher = RspackHash::with_salt(self.hash_function, self.hash_salt);
      self.filename.hash(&mut hasher);
      local.hash(&mut hasher);
      let hash = hasher.digest(self.hash_digest);
      let hash = hash.rendered(self.hash_digest_length);
      if hash.as_bytes()[0].is_ascii_digit() {
        format!("_{hash}")
      } else {
        hash.into()
      }
    };
    self
      .local_name_ident
      .render(LocalIdentNameRenderOptions {
        path_data: PathData::default()
          .filename(&self.filename.to_string_lossy())
          .hash(&hash),
        local: Some(local),
      })
      .into()
  }
}

pub fn css_modules_exports_to_string(
  exports: &IndexMap<JsWord, Vec<CssClassName>>,
  module: &dyn rspack_core::Module,
  compilation: &Compilation,
  locals_convention: &LocalsConvention,
) -> Result<String> {
  let mut code = String::from("module.exports = {\n");
  for (key, elements) in exports {
    let content = elements
      .iter()
      .map(|element| match element {
        CssClassName::Local { name } | CssClassName::Global { name } => {
          serde_json::to_string(&name.value).expect("TODO:")
        }
        CssClassName::Import { name, from } => {
          let name = serde_json::to_string(&name.value).expect("TODO:");

          let from = compilation
            .module_graph
            .module_graph_module_by_identifier(&module.identifier())
            .and_then(|mgm| {
              // workaround
              mgm.dependencies.iter().find_map(|id| {
                let dependency = compilation.module_graph.dependency_by_id(id);
                if let Some(dependency) = dependency {
                  if dependency.request() == from {
                    return compilation
                      .module_graph
                      .module_graph_module_by_dependency_id(id);
                  }
                }
                None
              })
            })
            .expect("should have css from module");

          let from = serde_json::to_string(from.id(&compilation.chunk_graph)).expect("TODO:");
          format!("{}({from})[{name}]", RuntimeGlobals::REQUIRE)
        }
      })
      .collect::<Vec<_>>()
      .join(" + \" \" + ");
    if locals_convention.as_is() {
      writeln!(
        code,
        "  {}: {},",
        serde_json::to_string(&key).expect("TODO:"),
        content,
      )
      .map_err(|e| internal_error!(e.to_string()))?;
    }
    if locals_convention.camel_case() {
      writeln!(
        code,
        "  {}: {},",
        serde_json::to_string(&key.to_lower_camel_case()).expect("TODO:"),
        content,
      )
      .map_err(|e| internal_error!(e.to_string()))?;
    }
    if locals_convention.dashes() {
      writeln!(
        code,
        "  {}: {},",
        serde_json::to_string(&key.to_kebab_case()).expect("TODO:"),
        content,
      )
      .map_err(|e| internal_error!(e.to_string()))?;
    }
  }
  code += "};\n";
  Ok(code)
}

static STRING_MULTILINE: Lazy<Regex> =
  Lazy::new(|| Regex::new(r"\\[\n\r\f]").expect("Invalid RegExp"));

static TRIM_WHITE_SPACES: Lazy<Regex> =
  Lazy::new(|| Regex::new(r"(^[ \t\n\r\f]*|[ \t\n\r\f]*$)").expect("Invalid RegExp"));

static UNESCAPE: Lazy<Regex> =
  Lazy::new(|| Regex::new(r"\\([0-9a-fA-F]{1,6}[ \t\n\r\f]?|[\s\S])").expect("Invalid RegExp"));

static DATA: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(?i)data:").expect("Invalid RegExp"));

pub fn normalize_url(s: &str) -> String {
  let result = STRING_MULTILINE.replace_all(s, "");
  let result = TRIM_WHITE_SPACES.replace_all(&result, "");
  let result = UNESCAPE.replace_all(&result, |caps: &Captures| {
    caps
      .get(0)
      .and_then(|m| {
        let m = m.as_str();
        if m.len() > 2 {
          if let Ok(r_u32) = u32::from_str_radix(m[1..].trim(), 16) {
            if let Some(ch) = char::from_u32(r_u32) {
              return Some(format!("{}", ch));
            }
          }
          None
        } else {
          Some(m[1..2].to_string())
        }
      })
      .unwrap_or(caps[0].to_string())
  });

  if DATA.is_match(&result) {
    return result.to_string();
  }
  if result.contains('%') {
    if let Ok(r) = urlencoding::decode(&result) {
      return r.to_string();
    }
  }

  result.to_string()
}
