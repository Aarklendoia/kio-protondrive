# QTest unit tests for the pure-logic pieces of worker/ that don't need a
# live D-Bus session, a running proton-drive CLI, or the plugin loader (see
# issue #90) — kept as a handful of small, standalone executables (one per
# tested unit, KDE/Qt convention: `tst_<thing>`) rather than one combined
# binary, so a failure's name already tells you which unit broke.
function(protondrive_add_test name)
    ecm_add_test(${name}.cpp
        TEST_NAME ${name}
        LINK_LIBRARIES Qt6::Test KF6::I18n
    )
    # The tested headers (photo_categories.h, shareable.h) live one
    # directory up, in worker/ itself — not installed anywhere, so this is
    # the only way to reach them.
    target_include_directories(${name} PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/..)
endfunction()

protondrive_add_test(tst_photo_categories)
protondrive_add_test(tst_shareable)
