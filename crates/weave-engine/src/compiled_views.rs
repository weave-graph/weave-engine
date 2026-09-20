//! Source-aware immutable view registrations; artifacts never grant authority.
use super::*;
impl Engine {
    pub(crate) fn initialize_compiled_views(&self) -> Result<()> {
        let has_column: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('live_views') WHERE name='source_digest')", [], |r| r.get(0))?;
        if !has_column {
            self.conn
                .execute_batch("ALTER TABLE live_views ADD COLUMN source_digest TEXT;")?;
        }
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS view_sources(id TEXT NOT NULL,principal TEXT NOT NULL,template TEXT NOT NULL,definition_digest TEXT NOT NULL,PRIMARY KEY(id,principal));")?;
        Ok(())
    }
    pub fn register_compiled_view(
        &mut self,
        instance_id: &str,
        template: &CompiledViewTemplate,
        tick: Option<i64>,
        host: &HostContext,
    ) -> Result<ViewSnapshot> {
        if !valid_id(instance_id) {
            return Err(err("E_VIEW", "view and principal IDs required"));
        }
        view_registration::validate_template(template).map_err(|d| err(&d.code, &d.message))?;
        self.register_view_inner(
            &ViewDefinition {
                id: instance_id.into(),
                expression: template.expression.clone(),
                clock: template.clock.clone(),
            },
            tick,
            host,
            Some(template),
        )
    }
    /// Read and check both binding copies before using source identities. A legacy row
    /// cannot become compiled merely because its expression happens to match.
    pub(crate) fn compiled_view_template(
        &self,
        definition: &ViewDefinition,
        host: &HostContext,
    ) -> Result<Option<CompiledViewTemplate>> {
        self.read_budget.request()?;
        type Row = (Option<String>, Option<String>, Option<String>);
        let row:Option<Row> = self.conn.query_row("SELECT substr(v.source_digest,1,72),CASE WHEN length(CAST(s.template AS BLOB))<=?3 THEN s.template END,substr(s.definition_digest,1,72) FROM live_views v LEFT JOIN view_sources s ON s.id=v.id AND s.principal=v.principal WHERE v.id=?1 AND v.principal=?2",params![definition.id,host.principal,self.read_budget.remaining().min(1024*1024) as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        match row {
            None | Some((None, None, None)) => Ok(None),
            Some((Some(marker), Some(encoded), Some(digest))) => {
                self.read_budget.charge(encoded.len())?;
                let template: CompiledViewTemplate = serde_json::from_str(&encoded)?;
                view_registration::validate_template(&template)
                    .map_err(|_| err("E_INTEGRITY", "compiled view source binding unavailable"))?;
                if marker != digest
                    || digest != template.definition_digest
                    || definition.expression != template.expression
                    || definition.clock != template.clock
                {
                    return Err(err("E_INTEGRITY", "compiled view source binding mismatch"));
                }
                Ok(Some(template))
            }
            _ => Err(err(
                "E_INTEGRITY",
                "compiled view source binding unavailable",
            )),
        }
    }
    pub(crate) fn view_definition_fingerprint(
        &self,
        definition: &ViewDefinition,
        host: &HostContext,
    ) -> Result<String> {
        let template = self.compiled_view_template(definition, host)?;
        selection::fingerprint(&(definition, template.as_ref().map(|t| &t.definition_digest)))
    }
    pub(crate) fn attach_view_sources(
        &self,
        definition: &ViewDefinition,
        host: &HostContext,
        result: &mut QueryResult,
    ) -> Result<()> {
        if let Some(template) = self.compiled_view_template(definition, host)? {
            merge_sources(&mut result.source_revisions, &template.source_revisions)?;
        }
        json_size(result, MATERIALIZED_LIMIT)?;
        Ok(())
    }
    /// Required-current read with definition/time checks in the same SQL snapshot.
    pub fn read_current_view(
        &self,
        selection: &CurrentViewSelection,
        host: &HostContext,
    ) -> Result<ViewSnapshot> {
        self.read_current_view_inner(selection, host)
            .map_err(|error| {
                if error.code == "E_UNAVAILABLE" {
                    err("E_UNAVAILABLE", "view unavailable")
                } else {
                    error
                }
            })
    }
    fn read_current_view_inner(
        &self,
        selection: &CurrentViewSelection,
        host: &HostContext,
    ) -> Result<ViewSnapshot> {
        let _read = self.read_budget.enter();
        if !valid_id(&selection.view_id)
            || !valid_id(&host.principal)
            || !view_registration::is_definition_digest(&selection.definition_digest)
        {
            return Err(err("E_UNAVAILABLE", "view unavailable"));
        }
        let tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        let (definition, _) = self.view_schedule_definition(&selection.view_id, host)?;
        let template = self
            .compiled_view_template(&definition, host)?
            .ok_or_else(|| err("E_UNAVAILABLE", "view unavailable"))?;
        let tick = match (&definition.clock, &selection.time) {
            (ViewClock::Fixed, ViewReadTime::Fixed) => None,
            (ViewClock::Tick, ViewReadTime::Tick { valid_at }) => Some(*valid_at),
            _ => return Err(err("E_UNAVAILABLE", "view unavailable")),
        };
        if template.definition_digest != selection.definition_digest {
            return Err(err("E_UNAVAILABLE", "view unavailable"));
        }
        let result = self.read_view(
            &selection.view_id,
            tick,
            ViewFreshness::RequireCurrent,
            host,
        )?;
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(result)
    }
}
