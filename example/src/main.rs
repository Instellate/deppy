use deppy_macros::Injectable;
use deppy::{Injectable as InjectableTrait, ServiceCollectionBuilder, ServiceHandler, ServiceScope};
use std::sync::Arc;

struct Funny {
    msg: String,
}

impl InjectableTrait for Funny {
    fn inject<T: ServiceHandler>(_: &T) -> Self {
        Funny {
            msg: String::from("This is funny!"),
        }
    }
}

#[derive(Injectable)]
#[injectable_config(post_init = Self::post_init)]
struct Funnier {
    funny: Arc<Funny>,
}

impl Funnier {
    fn print_smth(&self) {
        println!("Message: \"{}\"", self.funny.msg);
    }
    
    fn post_init(&self) {
        println!("Initialised!");
    }
}

fn main() {
    let mut builder = ServiceCollectionBuilder::default();
    builder.add_singleton::<Funny>().add_scoped::<Funnier>();
    let collection = builder.build();

    let funnier: Arc<Funnier> = collection.get_required_service();
    funnier.print_smth();
    
    let funnier: Arc<Funnier> = collection.get_required_service();
    funnier.print_smth();

    let scoped = ServiceScope::create(&collection);
    let funnier: Arc<Funnier> = scoped.get_required_service();
    funnier.print_smth();
    let funnier: Arc<Funnier> = scoped.get_required_service();
    funnier.print_smth();

    let scoped = ServiceScope::create(&collection);
    let funnier: Arc<Funnier> = scoped.get_required_service();
    funnier.print_smth();
    let funnier: Arc<Funnier> = scoped.get_required_service();
    funnier.print_smth();
}
