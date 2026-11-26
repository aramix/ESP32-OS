# Release builds (build_flash.sh --release): keep the home directory out of the
# images. __FILE__ in asserts and logs would otherwise name it. ESP-IDF passes
# its own flag files to the compiler and ignores CFLAGS, so the option has to be
# added here, for all components. Only set for release builds.
if(DEFINED ENV{ESP32OS_HIDE_HOME})
    idf_build_set_property(COMPILE_OPTIONS "-ffile-prefix-map=$ENV{ESP32OS_HIDE_HOME}=~" APPEND)
endif()
