use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::error::Error as ErrorTrait;
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use thiserror::Error as ThisError;

#[derive(Debug, ThisError)]
pub enum Error {
    #[error("Downcasting from any to type failed")]
    DowncastingFailed,
    #[error("Service couldn't be found")]
    ServiceNotFound,
    #[error("Custom error was provided")]
    CustomError(Box<dyn ErrorTrait>),
}

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
pub trait AsyncServiceHandler {
    async fn get_async_service_by_type_id(
        &self,
        type_id: &TypeId,
    ) -> Result<Arc<dyn Any + Send + Sync>, Error>;

    async fn get_async_service<T: Any + Send + Sync>(&self) -> Result<AsyncDep<T>, Error>
    where
        Self: Sized,
    {
        let any_arc = self
            .get_async_service_by_type_id(&TypeId::of::<T>())
            .await?;
        let converted_value = match any_arc.downcast::<T>() {
            Ok(v) => v,
            Err(_) => return Err(Error::DowncastingFailed),
        };
        Ok(AsyncDep(converted_value))
    }

    async fn get_required_async_service<T: Any + Send + Sync>(&self) -> AsyncDep<T>
    where
        Self: Sized,
    {
        self.get_async_service::<T>().await.unwrap()
    }
}

pub trait Injectable {
    fn inject<T: ServiceHandler>(handler: &T) -> Self;
}

#[async_trait]
pub trait AsyncInjectable {
    async fn inject<T: ServiceHandler + AsyncServiceHandler + Send + Sync>(
        handler: &T,
    ) -> Result<Self, Error>
    where
        Self: Sized;
}

/// Trait for initializing structs not owned by you.
/// Prefer `Injectable` when able to as it's less messy
pub trait Initialize<R: Any + Send + Sync> {
    fn initialize<T: ServiceHandler>(&self, handler: &T) -> R;
}

#[async_trait]
pub trait AsyncInitialize<R: Any + Send + Sync> {
    async fn initialize<T: ServiceHandler + AsyncServiceHandler + Send + Sync>(
        &self,
        handler: &T,
    ) -> Result<R, Error>;
}

#[derive(Clone)]
struct DefaultInitializer;

impl<I: Injectable + Any + Send + Sync> Initialize<I> for DefaultInitializer {
    fn initialize<T: ServiceHandler>(&self, handler: &T) -> I {
        I::inject(handler)
    }
}

#[async_trait]
impl<I: AsyncInjectable + Any + Send + Sync> AsyncInitialize<I> for DefaultInitializer {
    async fn initialize<T: ServiceHandler + AsyncServiceHandler + Send + Sync>(
        &self,
        handler: &T,
    ) -> Result<I, Error> {
        I::inject(handler).await
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

pub struct AsyncDep<T>(Arc<T>);

impl<T> Deref for AsyncDep<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub type InitializeFn<T> = Arc<dyn Fn(&T) -> Box<dyn Any + Send + Sync> + Send + Sync>;

#[derive(Clone)]
struct ServiceInformation {
    pub(crate) initialize_fn: InitializeFn<ServiceCollection>,
    pub(crate) initialize_async_fn: Option<Arc<dyn ToAny<ServiceCollection> + Send + Sync>>,
    pub(crate) type_: ServiceType,
}

#[derive(Clone)]
struct ScopedServiceInformation {
    pub(crate) initialize_fn: InitializeFn<ServiceScope>,
    pub(crate) initialize_async_fn: Option<Arc<dyn ToAny<ServiceScope> + Send + Sync>>,
    pub(crate) type_: ServiceType,
}

#[derive(Clone)]
pub struct ServiceCollection {
    service_info: Arc<HashMap<TypeId, ServiceInformation>>,
    scoped_service_info: Arc<HashMap<TypeId, ScopedServiceInformation>>,
    singletons: Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
}

#[async_trait]
trait ToAny<H: ServiceHandler + AsyncServiceHandler + Send + Sync> {
    async fn to_any(&self, handler: &H) -> Result<Arc<dyn Any + Send + Sync>, Error>;
}

struct DefaultToAny<T: Any + Send + Sync, I: AsyncInitialize<T> + Send + Sync>(
    Arc<I>,
    PhantomData<T>,
);

#[async_trait]
impl<
        T: Any + Send + Sync,
        I: AsyncInitialize<T> + Send + Sync,
        H: ServiceHandler + AsyncServiceHandler + Send + Sync,
    > ToAny<H> for DefaultToAny<T, I>
{
    async fn to_any(&self, handler: &H) -> Result<Arc<dyn Any + Send + Sync>, Error> {
        let val = self.0.initialize(handler).await?;
        Ok(Arc::new(val))
    }
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
    type ScopeType = ServiceScope;

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

    fn create_scope(&self) -> Self::ScopeType
    where
        Self::ScopeType: ServiceHandler,
    {
        Self::ScopeType::create(self)
    }
}

#[async_trait]
impl AsyncServiceHandler for ServiceCollection {
    async fn get_async_service_by_type_id(
        &self,
        type_id: &TypeId,
    ) -> Result<Arc<dyn Any + Send + Sync>, Error> {
        let information = self.service_info.get(type_id);

        if let Some(info) = information {
            match info.type_ {
                ServiceType::Singleton => self.get_singleton(type_id).ok_or(Error::ServiceNotFound),
                _ => {
                    let any = if let Some(a) = info.initialize_async_fn.as_ref() {
                        a.to_any(self).await?
                    } else {
                        (info.initialize_fn)(self).into()
                    };
                    Ok(any)
                }
            }
        } else {
            Err(Error::ServiceNotFound)
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

    async fn get_async_service(
        &self,
        type_id: &TypeId,
        type_: ServiceType,
    ) -> Result<Arc<dyn Any + Send + Sync>, Error> {
        let value = match type_ {
            ServiceType::Singleton => self
                .singletons
                .read()
                .map_err(|_| Error::ServiceNotFound)?
                .get(type_id)
                .cloned(),
            ServiceType::Scoped => self
                .scoped
                .read()
                .map_err(|_| Error::ServiceNotFound)?
                .get(type_id)
                .cloned(),
            ServiceType::Transient => {
                return self
                    .initialize_service(self.services.get(type_id).ok_or(Error::ServiceNotFound)?).await
            }
        };

        Err(Error::ServiceNotFound)
    }

    async fn initialize_service(
        &self,
        information: &ScopedServiceInformation,
    ) -> Result<Arc<dyn Any + Send + Sync>, Error> {
        if let Some(i) = information.initialize_async_fn.as_ref() {
            i.to_any(self).await
        } else {
            Ok((information.initialize_fn)(self).into())
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
    type ScopeType = Self;

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
        self.clone()
    }
}

#[async_trait]
impl AsyncServiceHandler for ServiceScope {
    async fn get_async_service_by_type_id(
        &self,
        type_id: &TypeId,
    ) -> Result<Arc<dyn Any + Send + Sync>, Error> {
        todo!()
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

#[derive(Default, Clone)]
pub struct ServiceCollectionBuilder {
    services: HashMap<TypeId, ServiceInformation>,
    scoped_services: HashMap<TypeId, ScopedServiceInformation>,
}

impl ServiceCollectionBuilder {
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

        let information = ServiceInformation {
            initialize_fn: collection_closure,
            initialize_async_fn: None,
            type_: type_.clone(),
        };

        let scoped_information = ScopedServiceInformation {
            initialize_fn: scoped_closure,
            initialize_async_fn: None,
            type_,
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

    pub fn build(self) -> ServiceCollection {
        ServiceCollection {
            service_info: Arc::new(self.services),
            scoped_service_info: Arc::new(self.scoped_services),
            singletons: Arc::new(Default::default()),
        }
    }
}
