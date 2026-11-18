//! Внешний мир системы тикетов: CRM, ERP и партнёры.
//!
//! CRM и ERP здесь — in-memory системы из крейта `external`, система тикетов говорит
//! с ними только через адаптеры ACL из `integration`. В настоящей компании счёт клиента
//! в CRM уже был бы; в демо его заводит сервер, когда регистрирует тенанта, — так,
//! как это сделал бы менеджер CRM.

use std::collections::HashMap;
use std::sync::Arc;

use external::{crm::InMemoryCrm, erp::InMemoryErp};
use integration::{
    crm::{CrmAdapter, new_account},
    erp::ErpAdapter,
};
use ticketing::application::partners::{Partner, PartnerRoster};
use ticketing::domain::{partner::PartnerId, sla::CustomerTier, tenant::TenantId};

pub type Crm = CrmAdapter<Arc<InMemoryCrm>>;
pub type Erp = ErpAdapter<Arc<InMemoryErp>>;

/// Партнёр по железу: забирает тикеты `Hardware`, когда на складе нет запчастей.
pub const FIXIT: PartnerId = PartnerId(1);
/// Партнёр по сетям: забирает тикеты `Network`.
pub const NETCO: PartnerId = PartnerId(2);

/// Демо-токены, с которыми партнёры обращаются к Open-Host Service.
pub const FIXIT_TOKEN: &str = "fixit-demo-token";
pub const NETCO_TOKEN: &str = "netco-demo-token";

#[derive(Debug, Clone)]
pub struct ExternalSystems {
    pub crm: Arc<InMemoryCrm>,
    pub erp: Arc<InMemoryErp>,
    pub partners: PartnerRoster,
    /// Токен доступа к Open-Host Service → партнёр.
    pub partner_tokens: HashMap<String, PartnerId>,
}

impl ExternalSystems {
    /// Мир для демонстрации: свободных запчастей для железа нет — такие тикеты уходят
    /// партнёру, сетевое оборудование есть — сетевые тикеты решают своими силами.
    pub fn demo() -> Self {
        let erp = InMemoryErp::default();
        erp.set_stock("MG-HW", 2, 2);
        erp.set_stock("MG-NET", 5, 1);
        Self {
            crm: Arc::new(InMemoryCrm::default()),
            erp: Arc::new(erp),
            partners: PartnerRoster::new(vec![
                Partner::new(FIXIT, "FixIt Hardware", &["Hardware"]),
                Partner::new(NETCO, "NetCo", &["Network"]),
            ]),
            partner_tokens: HashMap::from([
                (FIXIT_TOKEN.to_owned(), FIXIT),
                (NETCO_TOKEN.to_owned(), NETCO),
            ]),
        }
    }

    pub fn crm_adapter(&self) -> Crm {
        CrmAdapter::new(self.crm.clone())
    }

    pub fn erp_adapter(&self) -> Erp {
        ErpAdapter::new(self.erp.clone())
    }

    /// Заводит в CRM счёт клиента, стоящего за тенантом.
    pub fn open_account(&self, tenant_id: TenantId, name: &str, tier: CustomerTier) {
        self.crm.upsert_account(new_account(tenant_id, name, tier));
    }
}
