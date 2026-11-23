#include <Luau/AstQuery.h>
#include <Luau/Autocomplete.h>
#include <Luau/ToString.h>
#include <Luau/BuiltinDefinitions.h>
#include <Luau/ExperimentalFlags.h>
#include <Luau/Frontend.h>
#include <emscripten.h>
#include <cstring>
#include <unordered_map>

using namespace Luau;
LUAU_FASTFLAG(LuauSolverV2)

namespace {
struct Files final : FileResolver {
    std::unordered_map<std::string, std::string> sources;

    std::optional<SourceCode> readSource(const ModuleName& name) override {
        auto it = sources.find(name);
        if (it == sources.end()) return std::nullopt;
        return SourceCode{it->second, SourceCode::Module};
    }

    std::optional<ModuleInfo> resolveModule(const ModuleInfo*, AstExpr* node, const TypeCheckLimits&) override {
        if (auto value = node->as<AstExprConstantString>()) {
            std::string path(value->value.data, value->value.size);
            if (path.compare(0, 2, "./") == 0) {
                auto file = path.substr(2) + ".luau";
                if (sources.count(file)) return ModuleInfo{file};
            }
        }
        return std::nullopt;
    }
};
Files files;
NullConfigResolver config;
std::unique_ptr<Frontend> frontend;
// Definition-backed type functions refer to their declaring module via a weak pointer.
std::optional<LoadDefinitionFileResult> definition;
EM_JS(void, issue, (int line, const char* message), { Module.onIssue(line, UTF8ToString(message)); });
EM_JS(void, candidate, (const char* label), { Module.onLabel(UTF8ToString(label)); });
}

extern "C" {
EMSCRIPTEN_KEEPALIVE int init_analysis(const char* definitions) {
    // Match the pinned LSP's default set of non-experimental analysis flags.
    for (auto flag = FValue<bool>::list; flag; flag = flag->next)
        if (strncmp(flag->name, "Luau", 4) == 0 && !isAnalysisFlagExperimental(flag->name))
            flag->value = true;
    FFlag::LuauSolverV2.value = true;
    config.defaultConfig.mode = Mode::Strict;
    FrontendOptions options;
    options.retainFullTypeGraphs = true;
    frontend = std::make_unique<Frontend>(SolverMode::New, &files, &config, options);
    registerBuiltinGlobals(*frontend, frontend->globals);
    auto& globals = frontend->globals;
    auto loaded = frontend->loadDefinitionFile(globals, globals.globalScope, definitions, "@rustia", true);
    if (!loaded.success) {
        for (const auto& error : loaded.parseResult.errors)
            issue(error.getLocation().begin.line, error.what());
        if (loaded.module)
            for (const auto& error : loaded.module->errors)
                issue(error.location.begin.line, toString(error).c_str());
        return 0;
    }
    definition.emplace(std::move(loaded));
    freeze(globals.globalTypes);
    return 1;
}

EMSCRIPTEN_KEEPALIVE void set_source(const char* file, const char* text) {
    if (files.sources[file] != text) {
        files.sources[file] = text;
        frontend->markDirty(file);
    }
}

EMSCRIPTEN_KEEPALIVE void diagnostics(const char* file) {
    for (const auto& error : frontend->check(file).errors)
        if (error.moduleName == file)
            issue(error.location.begin.line, toString(error).c_str());
}

// Only identifiers can own hover help. AST bounds exclude nearby comments,
// literal strings and whitespace, even in a partially edited statement.
EMSCRIPTEN_KEEPALIVE const char* hover(const char* file, int line, int byteColumn) {
    frontend->check(file);
    auto source = frontend->getSourceModule(file);
    auto module = frontend->moduleResolver.getModule(file);
    if (!source || !module) return nullptr;

    Position pos{unsigned(line), unsigned(byteColumn)};
    AstExpr* expr = findExprAtPosition(*source, pos);
    if (!expr) return nullptr;
    Location name = expr->location;
    if (auto member = expr->as<AstExprIndexName>())
        name = member->indexLocation;
    else if (!expr->is<AstExprLocal>() && !expr->is<AstExprGlobal>())
        return nullptr;
    if (pos < name.begin || pos >= name.end) return nullptr;

    auto type = findTypeAtPosition(*module, *source, pos);
    if (!type) return nullptr;
    ToStringOptions options;
    options.functionTypeArguments = true;
    static std::string signature;
    signature = toString(*type, options);
    return signature.c_str();
}

EMSCRIPTEN_KEEPALIVE void complete(const char* file, int line, int byteColumn) {
    frontend->check(file);
    auto result = autocomplete(*frontend, file, Position{unsigned(line), unsigned(byteColumn)}, nullptr);
    for (const auto& entry : result.entryMap)
        candidate(entry.first.c_str());
}
}
