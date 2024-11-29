use deppy::{Dep, ServiceCollectionBuilder, ServiceHandler, ServiceScope};
use deppy_macros::Injectable;

#[derive(Injectable)]
struct Funny {
    #[injectable(default_value = "This is funny!")]
    msg: String,
}

#[derive(Injectable)]
#[injectable(post_init = Self::post_init)]
struct Funnier {
    funny: Dep<Funny>,
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

    let funnier: Dep<Funnier> = collection.get_required_service();
    funnier.print_smth();
    
    let funnier: Dep<Funnier> = collection.get_required_service();
    funnier.print_smth();

    let scoped = ServiceScope::create(&collection);
    let funnier: Dep<Funnier> = scoped.get_required_service();
    funnier.print_smth();
    let funnier: Dep<Funnier> = scoped.get_required_service();
    funnier.print_smth();

    let scoped = ServiceScope::create(&collection);
    let funnier: Dep<Funnier> = scoped.get_required_service();
    funnier.print_smth();
    let funnier: Dep<Funnier> = scoped.get_required_service();
    funnier.print_smth();
}
