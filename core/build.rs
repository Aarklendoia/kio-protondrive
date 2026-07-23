fn main() {
    // Generates the C++ side of the cxx bridge declared in src/bridge.rs.
    // The generated header/impl are consumed by worker/protondriveworker.cpp
    // via corrosion_add_cxxbridge() in the top-level CMakeLists.txt — this
    // build.rs only needs to produce the generated sources, not link them
    // (CMake links the final worker binary).
    cxx_build::bridge("src/bridge.rs")
        .std("c++17")
        .compile("protondrive-core-bridge");

    println!("cargo:rerun-if-changed=src/bridge.rs");
}
