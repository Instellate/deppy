use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;

pub trait ServiceHandler {
    type ScopeType: ServiceHandler;
    fn get_service_by_type_id(&self, type_id: &TypeId) -> Option<Arc<dyn Any + Send + Sync>>;

    fn create_scope(&self) -> Self::ScopeType
    where
        Self::ScopeType: ServiceHandler;

    fn get_service<T: Any + Send + Sync>(&self) -> Option<Dep<T>>
    where
        Self: Sized,
    {
        Some(Dep(self
            .get_service_by_type_id(&TypeId::of::<T>())?
            .downcast::<T>()
            .ok()?))
    }

    fn get_required_service<T: Any + Send + Sync>(&self) -> Dep<T>
    where
        Self: Sized,
    {
        self.get_service::<T>().unwrap()
    }
}

#[async_trait]
pub trait ServiceHandlerAsync<'l> {
    async fn get_async_service_by_type_id(
        &'l self,
        type_id: TypeId,
    ) -> Option<Arc<dyn Any + Send + Sync>>;

    async fn get_async_service<T: Any + Send + Sync>(&'l self) -> Option<Dep<T>>
    where
        Self: Sized,
    {
        Some(Dep(self
            .get_async_service_by_type_id(TypeId::of::<T>())
            .await?
            .downcast::<T>()
            .ok()?))
    }

    async fn get_required_async_service<T: Any + Send + Sync>(&'l self) -> Dep<T>
    where
        Self: Sized,
    {
        self.get_async_service::<T>().await.unwrap()
    }
}

pub trait Injectable {
    fn inject<T: ServiceHandler>(handler: &T) -> Self;
}

/// Trait for initializing structs not owned by you.
/// Prefer `Injectable` when able to as it's less messy
pub trait Initialize<R: Any + Send + Sync> {
    fn initialize<T: ServiceHandler>(&self, handler: &T) -> R;
}

#[async_trait]
pub trait InitializeAsync<R: Any + Send + Sync> {
    async fn initialize<T: ServiceHandler>(&self, handler: &T) -> R;
}

#[derive(Clone)]
struct DefaultInitializer;

impl<I: Injectable + Any + Send + Sync> Initialize<I> for DefaultInitializer {
    fn initialize<T: ServiceHandler>(&self, handler: &T) -> I {
        I::inject(handler)
    }
}

#[derive(Debug, Clone)]
pub enum ServiceType {
    Singleton,
    Scoped,
    Transient,
}

/// Used mainly by derive macro ``Injectable`` to identify what is considered a service and what is considered non-service
pub struct Dep<T>(Arc<T>);

impl<T> Deref for Dep<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

type InitializeFn<T> = Arc<dyn Fn(&T) -> Box<dyn Any + Send + Sync> + Send + Sync>;
type InitializeAsyncFn<'a, T> = Arc<
    dyn Fn(&'a T) -> Pin<Box<dyn Future<Output = Box<dyn Any + Send + Sync>> + Send + 'a>>
        + Send
        + Sync,
>;

#[derive(Clone)]
struct ServiceInformation<'a, T: ServiceHandler> {
    pub(crate) initialize_fn: InitializeFn<T>,
    pub(crate) initialize_async_fn: Option<InitializeAsyncFn<'a, T>>,
    pub(crate) type_: ServiceType,
}

#[derive(Clone)]
pub struct ServiceCollection<'a> {
    service_info: Arc<HashMap<TypeId, ServiceInformation<'a, ServiceCollection<'a>>>>,
    scoped_service_info: Arc<HashMap<TypeId, ServiceInformation<'a, ServiceScope<'a>>>>,
    singletons: Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
}

impl ServiceCollection<'_> {
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

impl<'a> ServiceHandler for ServiceCollection<'a> {
    type ScopeType = ServiceScope<'a>;

    fn get_service_by_type_id(&self, type_id: &TypeId) -> Option<Arc<dyn Any + Send + Sync>> {
        let info = self.service_info.get(type_id)?;

        match info.type_ {
            ServiceType::Singleton => self.get_singleton(type_id),
            _ => Some((info.initialize_fn)(self).into()),
        }
    }

    fn create_scope(&self) -> Self::ScopeType
    where
        Self::ScopeType: ServiceHandler,
    {
        Self::ScopeType::create(self)
    }
}

#[async_trait]
impl<'a> ServiceHandlerAsync<'a> for ServiceCollection<'a> {
    async fn get_async_service_by_type_id(
        &'a self,
        type_id: TypeId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        let info = self.service_info.get(&type_id)?;

        match info.type_ {
            ServiceType::Singleton => self.get_singleton(&type_id),
            _ => match info.initialize_async_fn.as_ref() {
                Some(i) => Some(i(self).await.into()),
                None => Some((info.initialize_fn)(self).into()),
            },
        }
    }
}

#[derive(Clone)]
pub struct ServiceScope<'a> {
    services: Arc<HashMap<TypeId, ServiceInformation<'a, ServiceScope<'a>>>>,
    singletons: Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
    scoped: Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
}

impl<'a> ServiceScope<'a> {
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

    async fn get_async_service(
        &'a self,
        type_id: TypeId,
        type_: ServiceType,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        let value = match type_ {
            ServiceType::Singleton => self.singletons.read().ok()?.get(&type_id).cloned(),
            ServiceType::Scoped => self.scoped.read().ok()?.get(&type_id).cloned(),
            ServiceType::Transient => {
                return Some((self.services.get(&type_id)?.initialize_fn)(self).into())
            }
        };

        if let Some(v) = value {
            Some(v)
        } else {
            let information = self.services.get(&type_id)?;

            let value: Arc<dyn Any + Send + Sync> = match information.initialize_async_fn.as_ref() {
                Some(i) => i(self).await.into(),
                None => (information.initialize_fn)(self).into(),
            };

            match type_ {
                ServiceType::Singleton => {
                    self.singletons.write().ok()?.insert(type_id, value.clone())
                }
                ServiceType::Scoped => self.scoped.write().ok()?.insert(type_id, value.clone()),
                ServiceType::Transient => panic!(),
            };

            Some(value)
        }
    }

    pub fn create(handler: &ServiceCollection<'a>) -> Self {
        Self {
            services: handler.scoped_service_info.clone(),
            singletons: handler.singletons.clone(),
            scoped: Arc::new(Default::default()),
        }
    }
}

impl<'a> ServiceHandler for ServiceScope<'a> {
    type ScopeType = ServiceScope<'a>;

    fn get_service_by_type_id(&self, type_id: &TypeId) -> Option<Arc<dyn Any + Send + Sync>> {
        let information = self.services.get(type_id);

        if let Some(info) = information {
            self.get_service(type_id, info.type_.clone())
        } else {
            None
        }
    }

    fn create_scope(&self) -> Self::ScopeType
    where
        Self::ScopeType: ServiceHandler,
    {
        todo!("Cloning cannot be done here for lifetime reasons")
    }
}

#[async_trait]
impl<'a> ServiceHandlerAsync<'a> for ServiceScope<'a> {
    async fn get_async_service_by_type_id(
        &'a self,
        type_id: TypeId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        let information = self.services.get(&type_id);

        if let Some(info) = information {
            self.get_async_service(type_id, info.type_.clone()).await
        } else {
            None
        }
    }
}

async fn cast_to_any<
    T: Any + Send + Sync,
    I: InitializeAsync<T> + Send + Sync + 'static,
    H: ServiceHandler + Send + Sync,
>(
    handler: &H,
    initializer: I,
) -> Box<dyn Any + Send + Sync> {
    Box::new(initializer.initialize(handler).await)
}

impl<'a> From<ServiceCollection<'a>> for ServiceScope<'a> {
    fn from(value: ServiceCollection<'a>) -> Self {
        Self {
            services: value.scoped_service_info,
            singletons: value.singletons,
            scoped: Arc::new(Default::default()),
        }
    }
}

#[derive(Default, Clone)]
pub struct ServiceCollectionBuilder<'a> {
    services: HashMap<TypeId, ServiceInformation<'a, ServiceCollection<'a>>>,
    scoped_services: HashMap<TypeId, ServiceInformation<'a, ServiceScope<'a>>>,
}

impl<'a> ServiceCollectionBuilder<'a> {
    pub fn add_service<T: Any + Send + Sync, I: Initialize<T> + Clone + Send + Sync + 'static>(
        mut self,
        type_: ServiceType,
        initializer: I,
    ) -> Self {
        let closure_clone = initializer.clone();
        let collection_closure: InitializeFn<ServiceCollection> =
            Arc::new(move |x| Box::new(closure_clone.initialize(x)));
        let scoped_closure: InitializeFn<ServiceScope> =
            Arc::new(move |x| Box::new(initializer.initialize(x)));

        let information = ServiceInformation::<ServiceCollection> {
            initialize_fn: collection_closure,
            type_: type_.clone(),
            initialize_async_fn: None,
        };

        let scoped_information = ServiceInformation::<ServiceScope> {
            initialize_fn: scoped_closure,
            type_,
            initialize_async_fn: None,
        };

        self.services.insert(TypeId::of::<T>(), information);
        self.scoped_services
            .insert(TypeId::of::<T>(), scoped_information);

        self
    }

    pub fn add_singleton<T: Injectable + Any + Send + Sync>(self) -> Self {
        self.add_service::<T, DefaultInitializer>(ServiceType::Singleton, DefaultInitializer)
    }

    pub fn add_scoped<T: Injectable + Any + Send + Sync>(self) -> Self {
        self.add_service::<T, DefaultInitializer>(ServiceType::Scoped, DefaultInitializer)
    }

    pub fn add_transient<T: Injectable + Any + Send + Sync>(self) -> Self {
        self.add_service::<T, DefaultInitializer>(ServiceType::Transient, DefaultInitializer)
    }

    pub fn build(self) -> ServiceCollection<'a> {
        ServiceCollection {
            service_info: Arc::new(self.services),
            scoped_service_info: Arc::new(self.scoped_services),
            singletons: Arc::new(Default::default()),
        }
    }

    pub fn add_async_service<
        T: Any + Send + Sync,
        I: InitializeAsync<T> + Clone + Send + Sync + 'static,
    >(
        mut self,
        type_: ServiceType,
        initializer: I,
    ) -> Self {
        let closure_clone = initializer.clone();
        let async_closure_clone = initializer.clone();
        let async_closure_clone2 = initializer.clone();

        let async_collection_closure: InitializeAsyncFn<ServiceCollection> =
            Arc::new(move |x| Box::pin(cast_to_any(x, async_closure_clone.clone()))); // I have no idea why but it does not like moving it here and introducing a references causes a creation of a lifetime that we don't want. This is already a mess as is
        let async_scoped_closure: InitializeAsyncFn<ServiceScope> =
            Arc::new(move |x| Box::pin(cast_to_any(x, async_closure_clone2.clone())));

        let collection_closure: InitializeFn<ServiceCollection> =
            Arc::new(move |x| Box::new(futures::executor::block_on(initializer.initialize(x))));
        let scoped_closure: InitializeFn<ServiceScope> =
            Arc::new(move |x| Box::new(futures::executor::block_on(closure_clone.initialize(x))));

        let information = ServiceInformation::<ServiceCollection> {
            initialize_fn: collection_closure,
            type_: type_.clone(),
            initialize_async_fn: Some(async_collection_closure),
        };

        let scoped_information = ServiceInformation::<ServiceScope> {
            initialize_fn: scoped_closure,
            type_,
            initialize_async_fn: Some(async_scoped_closure),
        };

        self.services.insert(TypeId::of::<T>(), information);
        self.scoped_services
            .insert(TypeId::of::<T>(), scoped_information);

        self
    }
}
