fn main() {
    // Compile the .slint UI only when the GUI is being built.
    #[cfg(feature = "gui")]
    slint_build::compile("ui/app.slint").expect("failed to compile the Slint UI");
}
