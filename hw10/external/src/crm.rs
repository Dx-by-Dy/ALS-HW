//! CRM: счета клиентов и история их обращений.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

use kernel::status::TicketStatus;

/// Счёт клиента в терминах CRM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrmAccount {
    /// Номер счёта, например `ACC-000042`.
    pub acct_no: String,
    /// Наименование клиента; CRM хранит его как ввели, часто прописными.
    pub cust_nm: String,
    /// Адрес почты; пустая строка, если его нет.
    pub email_addr: String,
    /// Код уровня обслуживания: `GLD`, `SLV`, `BRZ`; пусто — уровень по умолчанию.
    pub svc_lvl_cd: String,
    /// Код состояния счёта: `A` — действует, `S` — приостановлен, `C` — закрыт.
    pub acct_stat_cd: String,
}

/// Запись в истории обращений клиента. По ней CRM пишет клиенту.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrmCaseActivity {
    pub acct_no: String,
    /// Внешний номер обращения, например `TKT-7`.
    pub case_ref: String,
    /// Код состояния: `NEW`, `RESOLVED`, `AUTO_CLOSED`, `REOPENED`.
    pub case_stat: String,
    /// Уложились ли в срок: `Y`, `N`; пусто, если обращение не закрыто.
    pub sla_met_flag: String,
    /// Обещанный срок в миллисекундах от эпохи; `0`, если его нет.
    pub due_ts_ms: u64,
    /// Когда это случилось, в миллисекундах от эпохи.
    pub actv_ts_ms: u64,
}

/// Ошибки CRM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrmFault {
    /// Сервис не отвечает.
    ServiceDown,
    NoSuchAccount(String),
    /// Код состояния обращения CRM не знает.
    UnknownCaseStatus(String),
    /// Такой переход запрещают общие правила статусов.
    InvalidCaseTransition {
        case_ref: String,
        case_stat: String,
    },
}

impl fmt::Display for CrmFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServiceDown => f.write_str("CRM-503 service down"),
            Self::NoSuchAccount(acct) => write!(f, "CRM-404 account {acct} not found"),
            Self::UnknownCaseStatus(stat) => write!(f, "CRM-422 unknown case status '{stat}'"),
            Self::InvalidCaseTransition {
                case_ref,
                case_stat,
            } => write!(f, "CRM-409 case {case_ref} cannot become {case_stat}"),
        }
    }
}

impl Error for CrmFault {}

/// Клиент CRM — так выглядел бы SDK поставщика.
pub trait CrmClient: Send + Sync {
    fn fetch_account(
        &self,
        acct_no: &str,
    ) -> impl Future<Output = Result<CrmAccount, CrmFault>> + Send;

    fn log_case_activity(
        &self,
        activity: CrmCaseActivity,
    ) -> impl Future<Output = Result<(), CrmFault>> + Send;
}

impl<C: CrmClient> CrmClient for Arc<C> {
    fn fetch_account(
        &self,
        acct_no: &str,
    ) -> impl Future<Output = Result<CrmAccount, CrmFault>> + Send {
        (**self).fetch_account(acct_no)
    }

    fn log_case_activity(
        &self,
        activity: CrmCaseActivity,
    ) -> impl Future<Output = Result<(), CrmFault>> + Send {
        (**self).log_case_activity(activity)
    }
}

/// CRM в памяти: счета, история обращений и переключатель «сервис упал».
#[derive(Debug, Default)]
pub struct InMemoryCrm {
    state: Mutex<CrmState>,
}

#[derive(Debug, Default)]
struct CrmState {
    accounts: BTreeMap<String, CrmAccount>,
    /// Текущий статус каждого обращения — по правилам Shared Kernel.
    cases: HashMap<String, TicketStatus>,
    activities: Vec<CrmCaseActivity>,
    down: bool,
}

impl InMemoryCrm {
    /// Заводит счёт или заменяет его — так это сделал бы менеджер в интерфейсе CRM.
    pub fn upsert_account(&self, account: CrmAccount) {
        self.state()
            .accounts
            .insert(account.acct_no.clone(), account);
    }

    pub fn set_down(&self, down: bool) {
        self.state().down = down;
    }

    /// Вся история обращений в порядке записи.
    pub fn activities(&self) -> Vec<CrmCaseActivity> {
        self.state().activities.clone()
    }

    fn state(&self) -> MutexGuard<'_, CrmState> {
        self.state.lock().expect("CRM state is never poisoned")
    }
}

/// Какой статус обращения означает код CRM.
fn case_status(code: &str) -> Option<TicketStatus> {
    match code {
        "NEW" | "REOPENED" => Some(TicketStatus::Open),
        "RESOLVED" | "AUTO_CLOSED" => Some(TicketStatus::Closed),
        _ => None,
    }
}

impl CrmClient for InMemoryCrm {
    async fn fetch_account(&self, acct_no: &str) -> Result<CrmAccount, CrmFault> {
        let state = self.state();
        if state.down {
            return Err(CrmFault::ServiceDown);
        }
        state
            .accounts
            .get(acct_no)
            .cloned()
            .ok_or_else(|| CrmFault::NoSuchAccount(acct_no.to_owned()))
    }

    /// Новое обращение начинается с `NEW`, дальше переходы проверяет Shared Kernel.
    async fn log_case_activity(&self, activity: CrmCaseActivity) -> Result<(), CrmFault> {
        let mut state = self.state();
        if state.down {
            return Err(CrmFault::ServiceDown);
        }
        if !state.accounts.contains_key(&activity.acct_no) {
            return Err(CrmFault::NoSuchAccount(activity.acct_no));
        }
        let next = case_status(&activity.case_stat)
            .ok_or_else(|| CrmFault::UnknownCaseStatus(activity.case_stat.clone()))?;
        let allowed = match state.cases.get(&activity.case_ref) {
            None => activity.case_stat == "NEW",
            Some(current) => activity.case_stat != "NEW" && current.can_become(next),
        };
        if !allowed {
            return Err(CrmFault::InvalidCaseTransition {
                case_ref: activity.case_ref,
                case_stat: activity.case_stat,
            });
        }
        state.cases.insert(activity.case_ref.clone(), next);
        state.activities.push(activity);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> CrmAccount {
        CrmAccount {
            acct_no: "ACC-000001".to_owned(),
            cust_nm: "ACME CORP".to_owned(),
            email_addr: "it@acme.example".to_owned(),
            svc_lvl_cd: "GLD".to_owned(),
            acct_stat_cd: "A".to_owned(),
        }
    }

    fn activity(case_stat: &str) -> CrmCaseActivity {
        CrmCaseActivity {
            acct_no: "ACC-000001".to_owned(),
            case_ref: "TKT-1".to_owned(),
            case_stat: case_stat.to_owned(),
            sla_met_flag: String::new(),
            due_ts_ms: 0,
            actv_ts_ms: 1_000,
        }
    }

    fn crm() -> InMemoryCrm {
        let crm = InMemoryCrm::default();
        crm.upsert_account(account());
        crm
    }

    #[tokio::test]
    async fn account_is_found_by_number() {
        let crm = crm();
        assert_eq!(crm.fetch_account("ACC-000001").await, Ok(account()));
        assert_eq!(
            crm.fetch_account("ACC-000002").await,
            Err(CrmFault::NoSuchAccount("ACC-000002".to_owned()))
        );
    }

    #[tokio::test]
    async fn case_follows_shared_status_rules() {
        let crm = crm();
        for stat in ["NEW", "RESOLVED", "REOPENED", "AUTO_CLOSED"] {
            assert_eq!(crm.log_case_activity(activity(stat)).await, Ok(()));
        }
        assert_eq!(crm.activities().len(), 4);
    }

    #[tokio::test]
    async fn case_starts_with_new_and_does_not_close_twice() {
        let crm = crm();
        let rejected = |stat: &str| CrmFault::InvalidCaseTransition {
            case_ref: "TKT-1".to_owned(),
            case_stat: stat.to_owned(),
        };
        assert_eq!(
            crm.log_case_activity(activity("RESOLVED")).await,
            Err(rejected("RESOLVED"))
        );
        crm.log_case_activity(activity("NEW")).await.unwrap();
        assert_eq!(
            crm.log_case_activity(activity("NEW")).await,
            Err(rejected("NEW"))
        );
        crm.log_case_activity(activity("RESOLVED")).await.unwrap();
        assert_eq!(
            crm.log_case_activity(activity("AUTO_CLOSED")).await,
            Err(rejected("AUTO_CLOSED"))
        );
        assert_eq!(crm.activities().len(), 2);
    }

    #[tokio::test]
    async fn unknown_codes_and_accounts_are_rejected() {
        let crm = crm();
        assert_eq!(
            crm.log_case_activity(activity("WIP")).await,
            Err(CrmFault::UnknownCaseStatus("WIP".to_owned()))
        );
        let foreign = CrmCaseActivity {
            acct_no: "ACC-999999".to_owned(),
            ..activity("NEW")
        };
        assert_eq!(
            crm.log_case_activity(foreign).await,
            Err(CrmFault::NoSuchAccount("ACC-999999".to_owned()))
        );
    }

    #[tokio::test]
    async fn service_down_answers_nothing() {
        let crm = crm();
        crm.set_down(true);
        assert_eq!(
            crm.fetch_account("ACC-000001").await,
            Err(CrmFault::ServiceDown)
        );
        assert_eq!(
            crm.log_case_activity(activity("NEW")).await,
            Err(CrmFault::ServiceDown)
        );
        crm.set_down(false);
        assert!(crm.fetch_account("ACC-000001").await.is_ok());
    }
}
