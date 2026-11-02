use super::*;
use hps_config::{SubscriptionRefresh, SubscriptionSave, SubscriptionsOverview};
impl HpsClient {
    pub async fn subscriptions(&self) -> Result<SubscriptionsOverview, ClientError> {
        self.get_json("api/v1/subscriptions").await
    }
    pub async fn save_subscription(
        &self,
        input: &SubscriptionSave,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        let id = super::segment(&input.configuration.id)?;
        self.post_body(&format!("api/v1/subscriptions/{id}/save"), input)
            .await
    }
    pub async fn delete_subscription(
        &self,
        id: &str,
        expected_revision: i64,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        let id = super::segment(id)?;
        self.post_body(
            &format!("api/v1/subscriptions/{id}/delete"),
            &hps_domain::ExpectedRevision { expected_revision },
        )
        .await
    }
    pub async fn refresh_subscription(
        &self,
        id: &str,
        input: &SubscriptionRefresh,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        let id = super::segment(id)?;
        self.post_body(&format!("api/v1/subscriptions/{id}/refresh"), input)
            .await
    }
}
