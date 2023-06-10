use std::sync::Arc;

use dashmap::DashMap;

use crate::{
    db::{order::MongoOrderOutput, PhItem},
    server::order::QueryOrdersMessage,
};

pub trait OrderCache: Send + Sync + 'static {
    fn get_orders(&self, message: &QueryOrdersMessage) -> Option<Vec<MongoOrderOutput>>;

    fn set_orders(&self, message: QueryOrdersMessage, order: Vec<MongoOrderOutput>);

    fn contains_orders(&self, message: &QueryOrdersMessage) -> bool;

    fn clear_orders(&self);
}

#[derive(Clone, Debug)]
pub struct MapCache {
    pub ph_item_cache: Arc<DashMap<String, PhItem>>,
    pub orders_cache: Arc<DashMap<QueryOrdersMessage, Vec<MongoOrderOutput>>>,
}

impl MapCache {
    pub fn new() -> Arc<Self> {
        let ph_item_cache: Arc<DashMap<String, PhItem>> = Arc::new(DashMap::new());
        let orders_cache: Arc<DashMap<QueryOrdersMessage, Vec<MongoOrderOutput>>> =
            Arc::new(DashMap::new());
        Arc::new(Self {
            ph_item_cache,
            orders_cache,
        })
    }
}

impl OrderCache for MapCache {
    fn get_orders(&self, message: &QueryOrdersMessage) -> Option<Vec<MongoOrderOutput>> {
        self.orders_cache.get(message).map(|i| i.to_owned())
    }

    fn set_orders(&self, message: QueryOrdersMessage, order: Vec<MongoOrderOutput>) {
        self.orders_cache.insert(message, order);
    }

    fn contains_orders(&self, message: &QueryOrdersMessage) -> bool {
        self.orders_cache.contains_key(message)
    }

    fn clear_orders(&self) {
        self.orders_cache.clear();
    }
}
