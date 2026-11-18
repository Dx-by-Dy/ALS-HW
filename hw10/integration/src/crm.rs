//! Адаптер CRM: справочник клиентов и сообщения клиентам.

use external::crm::{CrmAccount, CrmCaseActivity, CrmClient, CrmFault};
use ticketing::application::ports::{
    CustomerDirectory, CustomerNotice, CustomerNoticeKind, CustomerNotifier, CustomerProfile,
    ExternalError,
};
use ticketing::domain::{
    sla::CustomerTier,
    tenant::TenantId,
    ticket::{CloseReason, TicketId},
    time::Timestamp,
};

const SYSTEM: &str = "CRM";

/// Номер счёта CRM, соответствующий тенанту.
pub fn account_number(tenant_id: TenantId) -> String {
    format!("ACC-{:06}", tenant_id.0)
}

/// Номер обращения в CRM, соответствующий тикету.
pub fn case_reference(ticket_id: TicketId) -> String {
    format!("TKT-{}", ticket_id.0)
}

/// Код уровня обслуживания в CRM.
pub fn tier_code(tier: CustomerTier) -> &'static str {
    match tier {
        CustomerTier::Gold => "GLD",
        CustomerTier::Silver => "SLV",
        CustomerTier::Bronze => "BRZ",
    }
}

/// Уровень клиента по коду CRM. Пустой код означает уровень по умолчанию — Bronze.
pub fn tier_from_code(code: &str) -> Result<CustomerTier, ExternalError> {
    match code.trim().to_ascii_uppercase().as_str() {
        "GLD" => Ok(CustomerTier::Gold),
        "SLV" => Ok(CustomerTier::Silver),
        "BRZ" | "" => Ok(CustomerTier::Bronze),
        other => Err(rejected(format!("unknown service level code '{other}'"))),
    }
}

/// Счёт CRM для нового клиента — так его завёл бы менеджер CRM.
pub fn new_account(tenant_id: TenantId, name: &str, tier: CustomerTier) -> CrmAccount {
    CrmAccount {
        acct_no: account_number(tenant_id),
        cust_nm: name.to_owned(),
        email_addr: String::new(),
        svc_lvl_cd: tier_code(tier).to_owned(),
        acct_stat_cd: "A".to_owned(),
    }
}

/// Профиль клиента из счёта CRM.
pub fn to_profile(
    tenant_id: TenantId,
    account: &CrmAccount,
) -> Result<CustomerProfile, ExternalError> {
    let name = account.cust_nm.trim();
    if name.is_empty() {
        return Err(rejected(format!("account {} has no name", account.acct_no)));
    }
    let email = account.email_addr.trim();
    let active = match account.acct_stat_cd.trim() {
        "A" => true,
        "S" | "C" => false,
        other => {
            return Err(rejected(format!("unknown account status code '{other}'")));
        }
    };
    Ok(CustomerProfile {
        tenant_id,
        name: name.to_owned(),
        email: (!email.is_empty()).then(|| email.to_owned()),
        tier: tier_from_code(&account.svc_lvl_cd)?,
        active,
    })
}

/// Запись в истории обращений CRM для сообщения клиенту.
pub fn to_case_activity(notice: &CustomerNotice) -> CrmCaseActivity {
    let (case_stat, sla_met_flag, due_ts_ms) = match notice.kind {
        CustomerNoticeKind::Received { due_at } => ("NEW", "", millis(due_at)),
        CustomerNoticeKind::Reopened { due_at } => ("REOPENED", "", millis(due_at)),
        CustomerNoticeKind::Closed { reason, sla_met } => {
            let stat = match reason {
                CloseReason::Resolved => "RESOLVED",
                CloseReason::Inactivity => "AUTO_CLOSED",
            };
            (stat, if sla_met { "Y" } else { "N" }, 0)
        }
    };
    CrmCaseActivity {
        acct_no: account_number(notice.tenant_id),
        case_ref: case_reference(notice.ticket_id),
        case_stat: case_stat.to_owned(),
        sla_met_flag: sla_met_flag.to_owned(),
        due_ts_ms,
        actv_ts_ms: millis(notice.at),
    }
}

fn millis(at: Timestamp) -> u64 {
    at.0.saturating_mul(1000)
}

fn rejected(reason: String) -> ExternalError {
    ExternalError::Rejected {
        system: SYSTEM,
        reason,
    }
}

fn from_fault(fault: CrmFault) -> ExternalError {
    match fault {
        CrmFault::ServiceDown => ExternalError::Unavailable {
            system: SYSTEM,
            reason: fault.to_string(),
        },
        other => rejected(other.to_string()),
    }
}

/// Реализует порты [`CustomerDirectory`] и [`CustomerNotifier`] поверх клиента CRM.
#[derive(Debug, Clone)]
pub struct CrmAdapter<C> {
    client: C,
}

impl<C: CrmClient> CrmAdapter<C> {
    pub fn new(client: C) -> Self {
        Self { client }
    }
}

impl<C: CrmClient> CustomerDirectory for CrmAdapter<C> {
    async fn find_customer(
        &self,
        tenant_id: TenantId,
    ) -> Result<Option<CustomerProfile>, ExternalError> {
        match self.client.fetch_account(&account_number(tenant_id)).await {
            Ok(account) => to_profile(tenant_id, &account).map(Some),
            Err(CrmFault::NoSuchAccount(_)) => Ok(None),
            Err(fault) => Err(from_fault(fault)),
        }
    }
}

impl<C: CrmClient> CustomerNotifier for CrmAdapter<C> {
    async fn notify_customer(&self, notice: &CustomerNotice) -> Result<(), ExternalError> {
        self.client
            .log_case_activity(to_case_activity(notice))
            .await
            .map_err(from_fault)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use external::crm::InMemoryCrm;

    use super::*;

    const ACME: TenantId = TenantId(42);

    fn account(svc_lvl_cd: &str, acct_stat_cd: &str) -> CrmAccount {
        CrmAccount {
            acct_no: "ACC-000042".to_owned(),
            cust_nm: "  ACME CORP ".to_owned(),
            email_addr: "it@acme.example".to_owned(),
            svc_lvl_cd: svc_lvl_cd.to_owned(),
            acct_stat_cd: acct_stat_cd.to_owned(),
        }
    }

    fn notice(kind: CustomerNoticeKind) -> CustomerNotice {
        CustomerNotice {
            tenant_id: ACME,
            ticket_id: TicketId(7),
            kind,
            at: Timestamp(1_700),
        }
    }

    #[test]
    fn identifiers_are_translated_to_crm_numbers() {
        assert_eq!(account_number(TenantId(42)), "ACC-000042");
        assert_eq!(account_number(TenantId(1_234_567)), "ACC-1234567");
        assert_eq!(case_reference(TicketId(7)), "TKT-7");
    }

    #[test]
    fn account_becomes_customer_profile() {
        assert_eq!(
            to_profile(ACME, &account("SLV", "A")),
            Ok(CustomerProfile {
                tenant_id: ACME,
                name: "ACME CORP".to_owned(),
                email: Some("it@acme.example".to_owned()),
                tier: CustomerTier::Silver,
                active: true,
            })
        );
    }

    #[test]
    fn service_level_codes_map_to_tiers() {
        for (code, tier) in [
            ("GLD", CustomerTier::Gold),
            ("slv", CustomerTier::Silver),
            (" BRZ ", CustomerTier::Bronze),
            ("", CustomerTier::Bronze),
        ] {
            assert_eq!(tier_from_code(code), Ok(tier), "code '{code}'");
        }
        for tier in [
            CustomerTier::Gold,
            CustomerTier::Silver,
            CustomerTier::Bronze,
        ] {
            assert_eq!(tier_from_code(tier_code(tier)), Ok(tier));
        }
    }

    #[test]
    fn unknown_codes_are_rejected_not_guessed() {
        assert_eq!(
            to_profile(ACME, &account("PLT", "A")),
            Err(rejected("unknown service level code 'PLT'".to_owned()))
        );
        assert_eq!(
            to_profile(ACME, &account("GLD", "X")),
            Err(rejected("unknown account status code 'X'".to_owned()))
        );
    }

    #[test]
    fn suspended_and_closed_accounts_are_inactive() {
        for status in ["S", "C"] {
            let profile = to_profile(ACME, &account("GLD", status)).unwrap();
            assert!(!profile.active, "status {status}");
        }
    }

    #[test]
    fn blank_email_means_no_email_and_blank_name_is_rejected() {
        let no_email = CrmAccount {
            email_addr: "  ".to_owned(),
            ..account("GLD", "A")
        };
        assert_eq!(to_profile(ACME, &no_email).unwrap().email, None);
        let no_name = CrmAccount {
            cust_nm: " ".to_owned(),
            ..account("GLD", "A")
        };
        assert!(matches!(
            to_profile(ACME, &no_name),
            Err(ExternalError::Rejected { .. })
        ));
    }

    #[test]
    fn new_account_round_trips_to_the_same_profile() {
        let account = new_account(ACME, "Acme", CustomerTier::Gold);
        assert_eq!(account.acct_no, "ACC-000042");
        let profile = to_profile(ACME, &account).unwrap();
        assert_eq!(profile.tier, CustomerTier::Gold);
        assert_eq!(profile.name, "Acme");
        assert!(profile.active);
        assert_eq!(profile.email, None);
    }

    #[test]
    fn received_and_reopened_notices_carry_the_deadline_in_millis() {
        let received = to_case_activity(&notice(CustomerNoticeKind::Received {
            due_at: Timestamp(88_000),
        }));
        assert_eq!(
            received,
            CrmCaseActivity {
                acct_no: "ACC-000042".to_owned(),
                case_ref: "TKT-7".to_owned(),
                case_stat: "NEW".to_owned(),
                sla_met_flag: String::new(),
                due_ts_ms: 88_000_000,
                actv_ts_ms: 1_700_000,
            }
        );
        let reopened = to_case_activity(&notice(CustomerNoticeKind::Reopened {
            due_at: Timestamp(5),
        }));
        assert_eq!(reopened.case_stat, "REOPENED");
        assert_eq!(reopened.due_ts_ms, 5_000);
    }

    #[test]
    fn closing_notice_names_how_and_whether_sla_was_met() {
        let closed = |reason, sla_met| {
            let activity =
                to_case_activity(&notice(CustomerNoticeKind::Closed { reason, sla_met }));
            (
                activity.case_stat,
                activity.sla_met_flag,
                activity.due_ts_ms,
            )
        };
        assert_eq!(
            closed(CloseReason::Resolved, true),
            ("RESOLVED".to_owned(), "Y".to_owned(), 0)
        );
        assert_eq!(
            closed(CloseReason::Inactivity, false),
            ("AUTO_CLOSED".to_owned(), "N".to_owned(), 0)
        );
    }

    #[test]
    fn huge_timestamp_does_not_overflow_millis() {
        let activity = to_case_activity(&CustomerNotice {
            at: Timestamp(u64::MAX / 10),
            ..notice(CustomerNoticeKind::Received {
                due_at: Timestamp(u64::MAX),
            })
        });
        assert_eq!(activity.actv_ts_ms, u64::MAX);
        assert_eq!(activity.due_ts_ms, u64::MAX);
    }

    fn adapter() -> (Arc<InMemoryCrm>, CrmAdapter<Arc<InMemoryCrm>>) {
        let crm = Arc::new(InMemoryCrm::default());
        (crm.clone(), CrmAdapter::new(crm))
    }

    #[tokio::test]
    async fn directory_finds_customer_or_reports_unknown() {
        let (crm, adapter) = adapter();
        crm.upsert_account(account("GLD", "A"));
        let found = adapter.find_customer(ACME).await.unwrap().unwrap();
        assert_eq!(found.tier, CustomerTier::Gold);
        assert_eq!(adapter.find_customer(TenantId(1)).await, Ok(None));
    }

    #[tokio::test]
    async fn outage_is_unavailable_and_bad_data_is_rejected() {
        let (crm, adapter) = adapter();
        crm.upsert_account(account("PLT", "A"));
        assert!(matches!(
            adapter.find_customer(ACME).await,
            Err(ExternalError::Rejected { system: "CRM", .. })
        ));
        crm.set_down(true);
        assert_eq!(
            adapter.find_customer(ACME).await,
            Err(ExternalError::Unavailable {
                system: "CRM",
                reason: "CRM-503 service down".to_owned(),
            })
        );
    }

    #[tokio::test]
    async fn notices_reach_crm_case_history() {
        let (crm, adapter) = adapter();
        crm.upsert_account(account("GLD", "A"));
        let received = notice(CustomerNoticeKind::Received {
            due_at: Timestamp(2_000),
        });
        let closed = notice(CustomerNoticeKind::Closed {
            reason: CloseReason::Resolved,
            sla_met: true,
        });
        adapter.notify_customer(&received).await.unwrap();
        adapter.notify_customer(&closed).await.unwrap();
        let history: Vec<_> = crm.activities().into_iter().map(|a| a.case_stat).collect();
        assert_eq!(history, ["NEW", "RESOLVED"]);
    }

    #[tokio::test]
    async fn crm_refusal_is_reported_in_ticket_terms() {
        let (crm, adapter) = adapter();
        crm.upsert_account(account("GLD", "A"));
        let closed = notice(CustomerNoticeKind::Closed {
            reason: CloseReason::Resolved,
            sla_met: true,
        });
        assert_eq!(
            adapter.notify_customer(&closed).await,
            Err(rejected(
                "CRM-409 case TKT-7 cannot become RESOLVED".to_owned()
            ))
        );
    }
}
