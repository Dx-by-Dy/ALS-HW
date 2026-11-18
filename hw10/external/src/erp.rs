//! ERP: складские остатки. Поля названы так, как их называет сама ERP.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

/// Завод (склад), с которого работает служба поддержки.
pub const SERVICE_PLANT: &str = "1000";

/// Запрос остатка по группе материалов на заводе.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErpStockRequest {
    /// Завод (WERKS), например `1000`.
    pub werks: String,
    /// Группа материалов (MATKL), например `MG-HW`.
    pub matkl: String,
}

/// Остаток группы материалов.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErpStockLevel {
    pub matkl: String,
    /// Свободный запас (LABST). Может быть отрицательным, если склад ушёл в минус.
    pub labst: i64,
    /// Зарезервировано под другие заказы (RESB).
    pub resb: i64,
    /// Единица измерения (MEINS), например `PC`.
    pub meins: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErpFault {
    /// ERP не ответила за отведённое время.
    Timeout,
    UnknownPlant(String),
    UnknownMaterialGroup(String),
}

impl fmt::Display for ErpFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("ERP timeout"),
            Self::UnknownPlant(werks) => write!(f, "ERP: plant {werks} does not exist"),
            Self::UnknownMaterialGroup(matkl) => {
                write!(f, "ERP: material group {matkl} does not exist")
            }
        }
    }
}

impl Error for ErpFault {}

/// Клиент ERP — так выглядел бы SDK поставщика.
pub trait ErpClient: Send + Sync {
    fn read_stock(
        &self,
        request: ErpStockRequest,
    ) -> impl Future<Output = Result<ErpStockLevel, ErpFault>> + Send;
}

impl<C: ErpClient> ErpClient for Arc<C> {
    fn read_stock(
        &self,
        request: ErpStockRequest,
    ) -> impl Future<Output = Result<ErpStockLevel, ErpFault>> + Send {
        (**self).read_stock(request)
    }
}

/// ERP в памяти: остатки на одном заводе и переключатель «сервис упал».
#[derive(Debug, Default)]
pub struct InMemoryErp {
    state: Mutex<ErpState>,
}

#[derive(Debug, Default)]
struct ErpState {
    /// Группа материалов → (свободный запас, резерв).
    stock: HashMap<String, (i64, i64)>,
    down: bool,
}

impl InMemoryErp {
    /// Устанавливает остаток группы материалов — как это сделала бы инвентаризация.
    pub fn set_stock(&self, matkl: &str, labst: i64, resb: i64) {
        self.state().stock.insert(matkl.to_owned(), (labst, resb));
    }

    pub fn set_down(&self, down: bool) {
        self.state().down = down;
    }

    fn state(&self) -> MutexGuard<'_, ErpState> {
        self.state.lock().expect("ERP state is never poisoned")
    }
}

impl ErpClient for InMemoryErp {
    async fn read_stock(&self, request: ErpStockRequest) -> Result<ErpStockLevel, ErpFault> {
        let state = self.state();
        if state.down {
            return Err(ErpFault::Timeout);
        }
        if request.werks != SERVICE_PLANT {
            return Err(ErpFault::UnknownPlant(request.werks));
        }
        let &(labst, resb) = state
            .stock
            .get(&request.matkl)
            .ok_or_else(|| ErpFault::UnknownMaterialGroup(request.matkl.clone()))?;
        Ok(ErpStockLevel {
            matkl: request.matkl,
            labst,
            resb,
            meins: "PC".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(werks: &str, matkl: &str) -> ErpStockRequest {
        ErpStockRequest {
            werks: werks.to_owned(),
            matkl: matkl.to_owned(),
        }
    }

    #[tokio::test]
    async fn stock_is_read_by_plant_and_material_group() {
        let erp = InMemoryErp::default();
        erp.set_stock("MG-HW", 5, 2);
        assert_eq!(
            erp.read_stock(request(SERVICE_PLANT, "MG-HW")).await,
            Ok(ErpStockLevel {
                matkl: "MG-HW".to_owned(),
                labst: 5,
                resb: 2,
                meins: "PC".to_owned(),
            })
        );
    }

    #[tokio::test]
    async fn unknown_plant_group_and_outage_are_faults() {
        let erp = InMemoryErp::default();
        erp.set_stock("MG-HW", 5, 0);
        assert_eq!(
            erp.read_stock(request("2000", "MG-HW")).await,
            Err(ErpFault::UnknownPlant("2000".to_owned()))
        );
        assert_eq!(
            erp.read_stock(request(SERVICE_PLANT, "MG-XX")).await,
            Err(ErpFault::UnknownMaterialGroup("MG-XX".to_owned()))
        );
        erp.set_down(true);
        assert_eq!(
            erp.read_stock(request(SERVICE_PLANT, "MG-HW")).await,
            Err(ErpFault::Timeout)
        );
    }
}
