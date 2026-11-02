use super::*;
use hps_config::{
    GroupDelete, GroupPin, GroupStrategy, GroupUpsert, SelectionConfig, SelectionSave,
};
use hps_domain::{GroupsOverview, SelectionDecision, SelectionQuery};

impl Supervisor {
    fn group_policy(&self, expected: i64) -> Result<SelectionConfig, ApplicationError> {
        let current = self.current();
        if current.revision != expected {
            return Err(StorageError::Conflict.into());
        }
        Ok(current.base_config.proxy.selection.clone())
    }
    pub fn groups_overview(&self) -> GroupsOverview<SelectionConfig> {
        let current = self.current();
        GroupsOverview {
            running_revision: current.revision,
            configuration: current.config.proxy.selection.clone(),
            recent_choices: current.engine.group_last_choices(),
            default_selection: current.engine.selection(&SelectionQuery::default()),
        }
    }
    pub fn simulate_group(
        &self,
        query: SelectionQuery,
    ) -> Result<SelectionDecision, ApplicationError> {
        let current = self.current();
        if query
            .node
            .as_ref()
            .is_some_and(|s| !hps_config::valid_node_id(s))
            || (query.node.is_some() && query.group.is_some())
        {
            return Err(ApplicationError::Invalid);
        }
        if query
            .group
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 64)
            || query.inbound.as_ref().is_some_and(|s| {
                s.len() > 64 || !current.config.proxy.inbounds.iter().any(|i| &i.id == s)
            })
            || query
                .destination_host
                .as_ref()
                .is_some_and(|s| !hps_config::valid_host(s))
            || query.destination_port == Some(0)
            || query
                .after
                .as_ref()
                .is_some_and(|s| s.len() > 70 || !hps_config::valid_node_id(s))
            || query.limit.is_some_and(|n| !(1..=200).contains(&n))
        {
            return Err(ApplicationError::Invalid);
        }
        Ok(current.engine.selection(&query))
    }
    pub async fn edit_groups(
        self: &Arc<Self>,
        input: SelectionSave,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        let current = self.current();
        let workspace = self.store.workspace().await?;
        if current.revision != input.expected_revision
            || workspace.desired.revision.id != input.expected_revision
        {
            return Err(StorageError::Conflict.into());
        }
        let mut config = current.base_config.clone();
        config.proxy.selection = input.configuration;
        let revision = self
            .create_revision(ConfigInput {
                toml: toml::to_string(&config).map_err(|_| ApplicationError::Invalid)?,
                expected_revision: input.expected_revision,
                expected_draft_version: None,
                note: "Group selection policy".into(),
                use_draft_credentials: false,
            })
            .await?;
        self.activate(ConfigActivate {
            revision: revision.id,
            expected_revision: input.expected_revision,
            defer_until_restart: false,
        })
        .await
    }
    pub async fn upsert_group(
        self: &Arc<Self>,
        input: GroupUpsert,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        let mut configuration = self.group_policy(input.expected_revision)?;
        if let Some(group) = configuration
            .groups
            .iter_mut()
            .find(|g| g.id == input.group.id)
        {
            *group = input.group;
        } else {
            configuration.groups.push(input.group);
        }
        self.edit_groups(SelectionSave {
            configuration,
            expected_revision: input.expected_revision,
        })
        .await
    }
    pub async fn delete_group(
        self: &Arc<Self>,
        input: GroupDelete,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        let mut configuration = self.group_policy(input.expected_revision)?;
        let before = configuration.groups.len();
        configuration.groups.retain(|g| g.id != input.group_id);
        if configuration.groups.len() == before {
            return Err(ApplicationError::GroupUnavailable);
        }
        self.edit_groups(SelectionSave {
            configuration,
            expected_revision: input.expected_revision,
        })
        .await
    }
    pub async fn pin_group(
        self: &Arc<Self>,
        input: GroupPin,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        let mut configuration = self.group_policy(input.expected_revision)?;
        let group = configuration
            .groups
            .iter_mut()
            .find(|g| g.id == input.group_id)
            .ok_or(ApplicationError::GroupUnavailable)?;
        group.pinned_node = input.node_id;
        group.fallback_nodes = input.fallback_nodes;
        if group.pinned_node.is_none() && group.strategy == GroupStrategy::Fixed {
            group.strategy = GroupStrategy::RoundRobin;
        }
        self.edit_groups(SelectionSave {
            configuration,
            expected_revision: input.expected_revision,
        })
        .await
    }
}
