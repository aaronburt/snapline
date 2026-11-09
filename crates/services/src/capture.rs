use snapline_core::actions::ActionId;

use crate::{ServiceError, Services};

pub fn run(_action: &ActionId, _services: &Services) -> Result<(), ServiceError> {
    Err(ServiceError::NotYet)
}
