use super::{PreparedModule, PreparedTree};
use crate::synthesis::voice::{
    PhoneSet, ResolvedPhone, Result, VoiceError, resolve_with_overrides,
};

pub(crate) struct PhoneResolver<'a> {
    languages: Vec<&'a PhoneSet>,
    unified: &'a PhoneSet,
    overrides: &'a [PhoneSet],
}

impl<'a> PhoneResolver<'a> {
    pub(crate) fn resolve(&self, language: &str, token: &str) -> Result<ResolvedPhone<'a>> {
        resolve_with_overrides(
            &self.languages,
            self.unified,
            self.overrides,
            language,
            token,
        )
    }
}

impl PreparedTree {
    /// Keep the language table child order, separate from the unified
    /// table. Configuration's supported-language display order is not this order.
    pub fn resolve_phone(&self, language: &str, token: &str) -> Result<ResolvedPhone<'_>> {
        self.phone_resolver(&[])?.resolve(language, token)
    }

    pub(crate) fn phone_resolver<'a>(
        &'a self,
        overrides: &'a [PhoneSet],
    ) -> Result<PhoneResolver<'a>> {
        let tables: Vec<_> = self
            .nodes
            .iter()
            .filter_map(|node| match &node.module {
                PreparedModule::PhoneSet(table) => Some(table),
                _ => None,
            })
            .collect();
        let mut unified = tables.iter().filter(|table| table.name == "multiphone");
        let unified_table = *unified
            .next()
            .ok_or_else(|| VoiceError::new("Model has no unified phone table"))?;
        if unified.next().is_some() {
            return Err(VoiceError::new(
                "Model has multiple unified phone tables; routing is unresolved",
            ));
        }
        let languages: Vec<_> = tables
            .into_iter()
            .filter(|table| table.name != "multiphone")
            .collect();
        Ok(PhoneResolver {
            languages,
            unified: unified_table,
            overrides,
        })
    }
}
