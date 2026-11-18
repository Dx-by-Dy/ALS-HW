//! Приём тикета: шаг 1 бизнес-процесса — создать тикет, проверив клиента в CRM.
//!
//! Проверка идёт до командной стороны: актор выполняет команды по одной и не должен ждать
//! внешних систем. В команду попадает уже готовый уровень клиента.

use std::error::Error;
use std::fmt;

use super::{
    commands::CreateTicketCommand,
    ports::{CustomerDirectory, ExternalError},
};
use crate::domain::tenant::TenantId;

/// Почему тикет не приняли.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntakeError {
    /// CRM о таком клиенте не знает.
    UnknownCustomer(TenantId),
    /// Обслуживание клиента приостановлено или договор закрыт.
    InactiveCustomer(TenantId),
    /// CRM не ответила или ответила так, что ответ нельзя понять.
    Crm(ExternalError),
}

impl fmt::Display for IntakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCustomer(id) => write!(f, "{id} is not a customer in CRM"),
            Self::InactiveCustomer(id) => {
                write!(f, "{id} cannot open tickets: the CRM account is not active")
            }
            Self::Crm(e) => write!(f, "cannot check the customer: {e}"),
        }
    }
}

impl Error for IntakeError {}

/// Проверяет клиента в CRM и достраивает команду создания тикета его уровнем.
pub async fn admit_ticket<D: CustomerDirectory>(
    directory: &D,
    category: String,
    description: String,
    tenant_id: TenantId,
) -> Result<CreateTicketCommand, IntakeError> {
    let customer = directory
        .find_customer(tenant_id)
        .await
        .map_err(IntakeError::Crm)?
        .ok_or(IntakeError::UnknownCustomer(tenant_id))?;
    if !customer.active {
        return Err(IntakeError::InactiveCustomer(tenant_id));
    }
    Ok(CreateTicketCommand::new(
        category,
        description,
        tenant_id,
        customer.tier,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::CustomerProfile;
    use crate::domain::sla::CustomerTier;

    /// Справочник с одним ответом на любой запрос.
    struct Directory(Result<Option<CustomerProfile>, ExternalError>);

    impl CustomerDirectory for Directory {
        async fn find_customer(
            &self,
            _: TenantId,
        ) -> Result<Option<CustomerProfile>, ExternalError> {
            self.0.clone()
        }
    }

    const ACME: TenantId = TenantId(1);

    fn profile(tier: CustomerTier, active: bool) -> Directory {
        Directory(Ok(Some(CustomerProfile {
            tenant_id: ACME,
            name: "ACME CORP".to_owned(),
            email: None,
            tier,
            active,
        })))
    }

    async fn admit(directory: &Directory) -> Result<CreateTicketCommand, IntakeError> {
        admit_ticket(directory, "Hardware".to_owned(), "Disk".to_owned(), ACME).await
    }

    #[tokio::test]
    async fn active_customer_gets_ticket_with_tier_from_crm() {
        assert_eq!(
            admit(&profile(CustomerTier::Gold, true)).await,
            Ok(CreateTicketCommand::new(
                "Hardware",
                "Disk",
                ACME,
                CustomerTier::Gold
            ))
        );
    }

    #[tokio::test]
    async fn unknown_or_inactive_customer_is_not_admitted() {
        assert_eq!(
            admit(&Directory(Ok(None))).await,
            Err(IntakeError::UnknownCustomer(ACME))
        );
        assert_eq!(
            admit(&profile(CustomerTier::Gold, false)).await,
            Err(IntakeError::InactiveCustomer(ACME))
        );
    }

    #[tokio::test]
    async fn crm_failure_stops_intake() {
        let down = ExternalError::Unavailable {
            system: "CRM",
            reason: "down".to_owned(),
        };
        assert_eq!(
            admit(&Directory(Err(down.clone()))).await,
            Err(IntakeError::Crm(down))
        );
    }
}
