//! Адаптер ERP: хватает ли ресурсов на решение тикета.

use external::erp::{ErpClient, ErpFault, ErpStockLevel, ErpStockRequest, SERVICE_PLANT};
use ticketing::application::ports::{ExternalError, Inventory};
use ticketing::domain::ticket::ResourceAvailability;

const SYSTEM: &str = "ERP";

/// Группа материалов ERP, нужная для категории тикета; `None`, если категория решается
/// без материалов (консультация, доступы, счета) и спрашивать ERP не о чем.
pub fn material_group(category: &str) -> Option<&'static str> {
    match category.trim().to_lowercase().as_str() {
        "hardware" => Some("MG-HW"),
        "network" => Some("MG-NET"),
        _ => None,
    }
}

/// Хватает ли ресурсов по остатку ERP: свободный запас за вычетом резерва должен быть больше нуля.
/// Склад в минусе — ресурсов нет.
pub fn availability(level: &ErpStockLevel) -> ResourceAvailability {
    if level.labst.saturating_sub(level.resb) > 0 {
        ResourceAvailability::Available
    } else {
        ResourceAvailability::Unavailable
    }
}

fn from_fault(fault: ErpFault) -> ExternalError {
    let reason = fault.to_string();
    match fault {
        ErpFault::Timeout => ExternalError::Unavailable {
            system: SYSTEM,
            reason,
        },
        ErpFault::UnknownPlant(_) | ErpFault::UnknownMaterialGroup(_) => ExternalError::Rejected {
            system: SYSTEM,
            reason,
        },
    }
}

/// Реализует порт [`Inventory`] поверх клиента ERP.
#[derive(Debug, Clone)]
pub struct ErpAdapter<E> {
    client: E,
}

impl<E: ErpClient> ErpAdapter<E> {
    pub fn new(client: E) -> Self {
        Self { client }
    }
}

impl<E: ErpClient> Inventory for ErpAdapter<E> {
    async fn check_resources(&self, category: &str) -> Result<ResourceAvailability, ExternalError> {
        let Some(matkl) = material_group(category) else {
            return Ok(ResourceAvailability::Available);
        };
        let request = ErpStockRequest {
            werks: SERVICE_PLANT.to_owned(),
            matkl: matkl.to_owned(),
        };
        let level = self.client.read_stock(request).await.map_err(from_fault)?;
        Ok(availability(&level))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use external::erp::InMemoryErp;

    use super::*;

    fn level(labst: i64, resb: i64) -> ErpStockLevel {
        ErpStockLevel {
            matkl: "MG-HW".to_owned(),
            labst,
            resb,
            meins: "PC".to_owned(),
        }
    }

    #[test]
    fn categories_map_to_material_groups() {
        assert_eq!(material_group("Hardware"), Some("MG-HW"));
        assert_eq!(material_group("  network "), Some("MG-NET"));
        assert_eq!(material_group("Billing"), None);
        assert_eq!(material_group("Technical Issue"), None);
    }

    #[test]
    fn free_stock_decides_availability() {
        assert_eq!(availability(&level(5, 2)), ResourceAvailability::Available);
        assert_eq!(availability(&level(1, 0)), ResourceAvailability::Available);
        assert_eq!(
            availability(&level(3, 3)),
            ResourceAvailability::Unavailable
        );
        assert_eq!(
            availability(&level(0, 0)),
            ResourceAvailability::Unavailable
        );
        assert_eq!(
            availability(&level(-4, 0)),
            ResourceAvailability::Unavailable
        );
        assert_eq!(
            availability(&level(i64::MIN, i64::MAX)),
            ResourceAvailability::Unavailable
        );
    }

    /// ERP, которая запоминает запросы и отвечает заданным остатком.
    struct RecordingErp {
        requests: Mutex<Vec<ErpStockRequest>>,
        answer: Result<ErpStockLevel, ErpFault>,
    }

    impl ErpClient for RecordingErp {
        async fn read_stock(&self, request: ErpStockRequest) -> Result<ErpStockLevel, ErpFault> {
            self.requests.lock().unwrap().push(request);
            self.answer.clone()
        }
    }

    fn recording(answer: Result<ErpStockLevel, ErpFault>) -> Arc<RecordingErp> {
        Arc::new(RecordingErp {
            requests: Mutex::new(Vec::new()),
            answer,
        })
    }

    #[tokio::test]
    async fn request_is_built_in_erp_terms() {
        let erp = recording(Ok(level(1, 0)));
        let adapter = ErpAdapter::new(erp.clone());
        assert_eq!(
            adapter.check_resources("Network").await,
            Ok(ResourceAvailability::Available)
        );
        assert_eq!(
            *erp.requests.lock().unwrap(),
            [ErpStockRequest {
                werks: "1000".to_owned(),
                matkl: "MG-NET".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn category_without_materials_does_not_ask_erp() {
        let erp = recording(Err(ErpFault::Timeout));
        let adapter = ErpAdapter::new(erp.clone());
        assert_eq!(
            adapter.check_resources("Billing").await,
            Ok(ResourceAvailability::Available)
        );
        assert!(erp.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn erp_faults_are_reported_in_ticket_terms() {
        let timeout = ErpAdapter::new(recording(Err(ErpFault::Timeout)));
        assert_eq!(
            timeout.check_resources("Hardware").await,
            Err(ExternalError::Unavailable {
                system: "ERP",
                reason: "ERP timeout".to_owned(),
            })
        );
        let unknown = ErpAdapter::new(recording(Err(ErpFault::UnknownMaterialGroup(
            "MG-HW".to_owned(),
        ))));
        assert!(matches!(
            unknown.check_resources("Hardware").await,
            Err(ExternalError::Rejected { system: "ERP", .. })
        ));
    }

    #[tokio::test]
    async fn adapter_works_with_in_memory_erp() {
        let erp = Arc::new(InMemoryErp::default());
        erp.set_stock("MG-HW", 2, 0);
        erp.set_stock("MG-NET", 4, 4);
        let adapter = ErpAdapter::new(erp);
        assert_eq!(
            adapter.check_resources("Hardware").await,
            Ok(ResourceAvailability::Available)
        );
        assert_eq!(
            adapter.check_resources("Network").await,
            Ok(ResourceAvailability::Unavailable)
        );
    }
}
