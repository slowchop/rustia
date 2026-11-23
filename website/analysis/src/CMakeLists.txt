cmake_minimum_required(VERSION 3.16)
project(RustiaBrowserAnalysis LANGUAGES CXX C)
set(CMAKE_CXX_STANDARD 17)
set(LUAU_BUILD_CLI OFF CACHE BOOL "" FORCE)
set(LUAU_BUILD_TESTS OFF CACHE BOOL "" FORCE)
add_compile_options(-fwasm-exceptions)
add_subdirectory("${LUAU_SOURCE}" upstream EXCLUDE_FROM_ALL)
add_executable(analyzer analysis.cpp)
target_link_libraries(analyzer PRIVATE Luau.Analysis)
set_target_properties(analyzer PROPERTIES SUFFIX ".mjs")
target_link_options(analyzer PRIVATE --no-entry -fwasm-exceptions -sMODULARIZE=1
    -sEXPORT_ES6=1 -sENVIRONMENT=worker -sALLOW_MEMORY_GROWTH=1 -sFILESYSTEM=0
    -sEXPORT_EXCEPTION_HANDLING_HELPERS=1 -sSTACK_SIZE=4194304 "-sEXPORTED_RUNTIME_METHODS=['cwrap']")
