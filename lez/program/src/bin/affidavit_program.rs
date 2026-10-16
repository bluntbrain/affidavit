//! Guest entrypoint: the LEZ runtime hands every message to `handle_message`.

#[cfg(target_os = "zkvm")]
lee_core::actor_entrypoint!(raw affidavit_lez::handle_message);

#[cfg(not(target_os = "zkvm"))]
fn main() {
    eprintln!("affidavit_program is a LEZ guest; build it with lez/build-program.sh");
}
