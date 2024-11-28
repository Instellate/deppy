use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub trait ServiceHandler {
    fn get_service_by_type_id(&self, type_id: &TypeId) -> Option<Arc<dyn Any + Send + Sync>>;

    fn get_service<T: Any + Send + Sync>(&self) -> Option<Arc<T>>
    where
        Self: Sized,
    {
        self.get_service_by_type_id(&TypeId::of::<T>())?
            .downcast::<T>()
            .ok()
    }

    fn get_required_service<T: Any + Send + Sync>(&self) -> Arc<T>
    where
        Self: Sized,
    {
        self.get_service::<T>().unwrap()
    }
}

pub trait Injectable {
    fn inject<T: ServiceHandler>(handler: &T) -> Self;
}

#[derive(Debug, Clone)]
pub enum ServiceType {
    Singleton,
    Scoped,
    Transient,
}

pub type InitializeFn<T> = fn(&T) -> Box<dyn Any + Send + Sync>;

#[derive(Debug, Clone)]
struct ServiceInformation {
    pub(crate) initialize_fn: InitializeFn<ServiceCollection>,
    pub(crate) type_: ServiceType,
}

#[derive(Debug, Clone)]
struct ScopedServiceInformation {
    initialize_fn: InitializeFn<ServiceScope>,
    type_: ServiceType,
}

#[derive(Debug, Clone)]
pub struct ServiceCollection {
    service_info: Arc<HashMap<TypeId, ServiceInformation>>,
    scoped_service_info: Arc<HashMap<TypeId, ScopedServiceInformation>>,
    singletons: Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
}

impl ServiceCollection {
    fn get_singleton(&self, type_id: &TypeId) -> Option<Arc<dyn Any + Send + Sync>> {
        let value = {
            let read = self.singletons.read().ok()?;
            read.get(type_id).cloned()
        };

        if let Some(v) = value {
            Some(v)
        } else {
            let information = self.service_info.get(type_id)?;
            let value: Arc<dyn Any + Send + Sync> = (information.initialize_fn)(self).into();
            let mut write = self.singletons.write().ok()?;
            write.insert(*type_id, value.clone());
            Some(value)
        }
    }
}

impl ServiceHandler for ServiceCollection {
    fn get_service_by_type_id(&self, type_id: &TypeId) -> Option<Arc<dyn Any + Send + Sync>> {
        let information = self.service_info.get(type_id);

        if let Some(info) = information {
            match info.type_ {
                ServiceType::Singleton => Some(self.get_singleton(type_id)?),
                _ => Some((info.initialize_fn)(self).into()),
            }
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub struct ServiceScope {
    services: Arc<HashMap<TypeId, ScopedServiceInformation>>,
    singletons: Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
    scoped: Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
}

impl ServiceScope {
    fn get_service(
        &self,
        type_id: &TypeId,
        type_: ServiceType,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        let value = match type_ {
            ServiceType::Singleton => self.singletons.read().ok()?.get(type_id).cloned(),
            ServiceType::Scoped => self.scoped.read().ok()?.get(type_id).cloned(),
            ServiceType::Transient => {
                return Some((self.services.get(type_id)?.initialize_fn)(self).into())
            }
        };

        if let Some(v) = value {
            Some(v)
        } else {
            let information = self.services.get(type_id)?;
            let value: Arc<dyn Any + Send + Sync> = (information.initialize_fn)(self).into();

            match type_ {
                ServiceType::Singleton => self
                    .singletons
                    .write()
                    .ok()?
                    .insert(*type_id, value.clone()),
                ServiceType::Scoped => self.scoped.write().ok()?.insert(*type_id, value.clone()),
                ServiceType::Transient => panic!(),
            };

            Some(value)
        }
    }

    pub fn create(handler: &ServiceCollection) -> Self {
        Self {
            services: handler.scoped_service_info.clone(),
            singletons: handler.singletons.clone(),
            scoped: Arc::new(Default::default()),
        }
    }
}

impl ServiceHandler for ServiceScope {
    fn get_service_by_type_id(&self, type_id: &TypeId) -> Option<Arc<dyn Any + Send + Sync>> {
        let information = self.services.get(type_id);

        if let Some(info) = information {
            self.get_service(type_id, info.type_.clone())
        } else {
            None
        }
    }
}

impl From<ServiceCollection> for ServiceScope {
    fn from(value: ServiceCollection) -> Self {
        Self {
            services: value.scoped_service_info,
            singletons: value.singletons,
            scoped: Arc::new(Default::default()),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct ServiceCollectionBuilder {
    services: HashMap<TypeId, ServiceInformation>,
    scoped_services: HashMap<TypeId, ScopedServiceInformation>,
}

impl ServiceCollectionBuilder {
    pub fn add_service<T: Injectable + Any + Send + Sync>(
        &mut self,
        type_: ServiceType,
    ) -> &mut Self {
        let collection_closure: InitializeFn<ServiceCollection> = |x| Box::new(T::inject(x));
        let scoped_closure: InitializeFn<ServiceScope> = |x| Box::new(T::inject(x));

        let information = ServiceInformation {
            initialize_fn: collection_closure,
            type_: type_.clone(),
        };

        let scoped_information = ScopedServiceInformation {
            initialize_fn: scoped_closure,
            type_,
        };

        self.services.insert(TypeId::of::<T>(), information);
        self.scoped_services
            .insert(TypeId::of::<T>(), scoped_information);

        self
    }

    pub fn add_singleton<T: Injectable + Any + Send + Sync>(&mut self) -> &mut Self {
        self.add_service::<T>(ServiceType::Singleton)
    }

    pub fn add_scoped<T: Injectable + Any + Send + Sync>(&mut self) -> &mut Self {
        self.add_service::<T>(ServiceType::Scoped)
    }

    pub fn add_transient<T: Injectable + Any + Send + Sync>(&mut self) -> &mut Self {
        self.add_service::<T>(ServiceType::Transient)
    }

    pub fn build(self) -> ServiceCollection {
        ServiceCollection {
            service_info: Arc::new(self.services),
            scoped_service_info: Arc::new(self.scoped_services),
            singletons: Arc::new(Default::default()),
        }
    }
}
