#!/usr/bin/env python3
"""生成 app/Zhunji.xcodeproj/project.pbxproj（幂等，重跑安全）。

枚举 app/Sources 下的 .swift 文件自动纳入 target；Resources 目录下的
libzhunji_core.dylib 进 Frameworks 链接（build_core.sh 负责拷贝）。

以后加 Swift 文件 → 重跑本脚本即可。
"""
import os
import sys

APP_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "app"))
SOURCES_DIR = os.path.join(APP_DIR, "Sources")
RESOURCES_DIR = os.path.join(APP_DIR, "Resources")
PROJECT_DIR = os.path.join(APP_DIR, "Zhunji.xcodeproj")

# ── UUID 生成：AA00000000000000000000NN 递增，稳定幂等 ──
_counter = 0


def uid() -> str:
    global _counter
    _counter += 1
    return f"AA{_counter:022d}"


def collect_swift_files() -> list[str]:
    files = []
    for root, _, names in sorted(os.walk(SOURCES_DIR)):
        for name in sorted(names):
            if name.endswith(".swift"):
                files.append(os.path.relpath(os.path.join(root, name), SOURCES_DIR))
    if not files:
        print("错误: Sources 目录没有 .swift 文件", file=sys.stderr)
        sys.exit(1)
    return files


def collect_resources() -> list[str]:
    """Resources 目录下随 bundle 打包的资源（dylib 走 CopyFiles，不在此列）。"""
    files = []
    for name in sorted(os.listdir(RESOURCES_DIR)):
        path = os.path.join(RESOURCES_DIR, name)
        if os.path.isfile(path) and not name.endswith(".dylib"):
            files.append(name)
    return files


def main() -> None:
    swift_files = collect_swift_files()
    os.makedirs(PROJECT_DIR, exist_ok=True)

    # ── PBXFileReference ──
    file_refs = {}   # path -> uid
    build_files = []  # (uid, comment)
    for rel in swift_files:
        uid_fr = uid()
        file_refs[rel] = uid_fr
        build_files.append((uid(), rel, f"{os.path.basename(rel)} in Sources"))

    resources = collect_resources()
    res_refs = {}  # name -> uid
    res_build_files = {}  # name -> uid
    for name in resources:
        res_refs[name] = uid()
        res_build_files[name] = uid()

    dylib_ref = uid()
    dylib_bf = uid()
    app_ref = uid()
    info_ref = uid()
    ent_ref = uid()
    copy_files_phase = uid()

    groups = {
        "root": uid(),
        "sources": uid(),
        "frameworks": uid(),
        "products": uid(),
    }

    # ── Build phases ──
    sources_phase = uid()
    frameworks_phase = uid()
    resources_phase = uid()
    target = uid()
    project = uid()
    target_cfglist = uid()
    project_cfglist = uid()
    cfg_debug = uid()
    cfg_release = uid()
    cfg_debug_project = uid()
    cfg_release_project = uid()

    lines = [
        "// !$*UTF8*$!",
        "{",
        "\tarchiveVersion = 1;",
        "\tclasses = {",
        "\t};",
        "\tobjectVersion = 56;",
        "\tobjects = {",
        "",
        "/* Begin PBXBuildFile section */",
    ]
    for uid_bf, rel, comment in build_files:
        lines.append(f"\t\t{uid_bf} /* {comment} */ = {{isa = PBXBuildFile; fileRef = {file_refs[rel]} /* {os.path.basename(rel)} */; }};")
    lines.append(f"\t\t{dylib_bf} /* libzhunji_core.dylib in CopyFiles */ = {{isa = PBXBuildFile; fileRef = {dylib_ref} /* libzhunji_core.dylib */; settings = {{ATTRIBUTES = (CodeSignOnCopy, RemoveHeadersOnCopy, ); }}; }};")
    for name in resources:
        lines.append(f"\t\t{res_build_files[name]} /* {name} in Resources */ = {{isa = PBXBuildFile; fileRef = {res_refs[name]} /* {name} */; }};")
    lines.append("/* End PBXBuildFile section */")
    lines.append("")

    lines.append("/* Begin PBXFileReference section */")
    lines.append(f"\t\t{app_ref} /* Zhunji.app */ = {{isa = PBXFileReference; explicitFileType = wrapper.application; includeInIndex = 0; path = Zhunji.app; sourceTree = BUILT_PRODUCTS_DIR; }};")
    for rel in swift_files:
        lines.append(f"\t\t{file_refs[rel]} /* {os.path.basename(rel)} */ = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = {rel}; sourceTree = \"<group>\"; }};")
    lines.append(f"\t\t{dylib_ref} /* libzhunji_core.dylib */ = {{isa = PBXFileReference; lastKnownFileType = compiled.mach-o.dylib; name = libzhunji_core.dylib; path = Resources/libzhunji_core.dylib; sourceTree = \"<group>\"; }};")
    lines.append(f"\t\t{info_ref} /* Info.plist */ = {{isa = PBXFileReference; lastKnownFileType = text.plist.xml; path = Info.plist; sourceTree = \"<group>\"; }};")
    lines.append(f"\t\t{ent_ref} /* Entitlements.plist */ = {{isa = PBXFileReference; lastKnownFileType = text.plist.xml; path = Entitlements.plist; sourceTree = \"<group>\"; }};")
    for name in resources:
        lines.append(f"\t\t{res_refs[name]} /* {name} */ = {{isa = PBXFileReference; lastKnownFileType = file; path = Resources/{name}; sourceTree = \"<group>\"; }};")
    lines.append("/* End PBXFileReference section */")
    lines.append("")

    lines.append("/* Begin PBXFrameworksBuildPhase section */")
    lines.append(f"\t\t{frameworks_phase} /* Frameworks */ = {{")
    lines.append("\t\t\tisa = PBXFrameworksBuildPhase;")
    lines.append("\t\t\tbuildActionMask = 2147483647;")
    lines.append("\t\t\tfiles = (")
    lines.append("\t\t\t);")
    lines.append("\t\t\trunOnlyForDeploymentPostprocessing = 0;")
    lines.append("\t\t};")
    lines.append("/* End PBXFrameworksBuildPhase section */")
    lines.append("")

    lines.append("/* Begin PBXCopyFilesBuildPhase section */")
    lines.append(f"\t\t{copy_files_phase} /* Copy Frameworks */ = {{")
    lines.append("\t\t\tisa = PBXCopyFilesBuildPhase;")
    lines.append("\t\t\tbuildActionMask = 2147483647;")
    lines.append("\t\t\tdstPath = \"\";")
    lines.append("\t\t\tdstSubfolderSpec = 10;")
    lines.append("\t\t\tfiles = (")
    lines.append(f"\t\t\t\t{dylib_bf} /* libzhunji_core.dylib in CopyFiles */,")
    lines.append("\t\t\t);")
    lines.append("\t\t\tname = \"Copy Frameworks\";")
    lines.append("\t\t\trunOnlyForDeploymentPostprocessing = 0;")
    lines.append("\t\t};")
    lines.append("/* End PBXCopyFilesBuildPhase section */")
    lines.append("")
    lines.append("/* Begin PBXGroup section */")
    lines.append(f"\t\t{groups['root']} = {{")
    lines.append("\t\t\tisa = PBXGroup;")
    lines.append("\t\t\tchildren = (")
    lines.append(f"\t\t\t\t{groups['sources']} /* Sources */,")
    lines.append(f"\t\t\t\t{groups['frameworks']} /* Frameworks */,")
    lines.append(f"\t\t\t\t{groups['products']} /* Products */,")
    lines.append("\t\t\t);")
    lines.append("\t\t\tsourceTree = \"<group>\";")
    lines.append("\t\t};")
    lines.append(f"\t\t{groups['sources']} /* Sources */ = {{")
    lines.append("\t\t\tisa = PBXGroup;")
    lines.append("\t\t\tchildren = (")
    for rel in swift_files:
        lines.append(f"\t\t\t\t{file_refs[rel]} /* {os.path.basename(rel)} */,")
    lines.append(f"\t\t\t\t{info_ref} /* Info.plist */,")
    lines.append(f"\t\t\t\t{ent_ref} /* Entitlements.plist */,")
    lines.append("\t\t\t);")
    lines.append("\t\t\tpath = Sources;")
    lines.append("\t\t\tsourceTree = \"<group>\";")
    lines.append("\t\t};")
    lines.append(f"\t\t{groups['frameworks']} /* Frameworks */ = {{")
    lines.append("\t\t\tisa = PBXGroup;")
    lines.append("\t\t\tchildren = (")
    lines.append(f"\t\t\t\t{dylib_ref} /* libzhunji_core.dylib */,")
    lines.append("\t\t\t);")
    lines.append("\t\t\tname = Frameworks;")
    lines.append("\t\t\tsourceTree = \"<group>\";")
    lines.append("\t\t};")
    lines.append(f"\t\t{groups['products']} /* Products */ = {{")
    lines.append("\t\t\tisa = PBXGroup;")
    lines.append("\t\t\tchildren = (")
    lines.append(f"\t\t\t\t{app_ref} /* Zhunji.app */,")
    lines.append("\t\t\t);")
    lines.append("\t\t\tname = Products;")
    lines.append("\t\t\tsourceTree = \"<group>\";")
    lines.append("\t\t};")
    lines.append("/* End PBXGroup section */")
    lines.append("")

    lines.append("/* Begin PBXNativeTarget section */")
    lines.append(f"\t\t{target} /* Zhunji */ = {{")
    lines.append("\t\t\tisa = PBXNativeTarget;")
    lines.append(f"\t\t\tbuildConfigurationList = {target_cfglist} /* Build configuration list for PBXNativeTarget \"Zhunji\" */;")
    lines.append("\t\t\tbuildPhases = (")
    lines.append(f"\t\t\t\t{sources_phase} /* Sources */,")
    lines.append(f"\t\t\t\t{frameworks_phase} /* Frameworks */,")
    lines.append(f"\t\t\t\t{copy_files_phase} /* Copy Frameworks */,")
    lines.append(f"\t\t\t\t{resources_phase} /* Resources */,")
    lines.append("\t\t\t);")
    lines.append("\t\t\tbuildRules = (")
    lines.append("\t\t\t);")
    lines.append("\t\t\tdependencies = (")
    lines.append("\t\t\t);")
    lines.append("\t\t\tname = Zhunji;")
    lines.append(f"\t\t\tproductName = Zhunji;")
    lines.append(f"\t\t\tproductReference = {app_ref} /* Zhunji.app */;")
    lines.append("\t\t\tproductType = \"com.apple.product-type.application\";")
    lines.append("\t\t};")
    lines.append("/* End PBXNativeTarget section */")
    lines.append("")

    lines.append("/* Begin PBXProject section */")
    lines.append(f"\t\t{project} /* Project object */ = {{")
    lines.append("\t\t\tisa = PBXProject;")
    lines.append("\t\t\tattributes = {")
    lines.append("\t\t\t\tBuildIndependentTargetsInParallel = 1;")
    lines.append("\t\t\t\tLastSwiftUpdateCheck = 1600;")
    lines.append("\t\t\t\tLastUpgradeCheck = 1600;")
    lines.append("\t\t\t};")
    lines.append(f"\t\t\tbuildConfigurationList = {project_cfglist} /* Build configuration list for PBXProject \"Zhunji\" */;")
    lines.append("\t\t\tcompatibilityVersion = \"Xcode 14.0\";")
    lines.append("\t\t\tdevelopmentRegion = zh_CN;")
    lines.append("\t\t\thasScannedForEncodings = 0;")
    lines.append("\t\t\tknownRegions = (")
    lines.append("\t\t\t\ten,")
    lines.append("\t\t\t\tzh_CN,")
    lines.append("\t\t\t\tBase,")
    lines.append("\t\t\t);")
    lines.append(f"\t\t\tmainGroup = {groups['root']};")
    lines.append(f"\t\t\tproductRefGroup = {groups['products']} /* Products */;")
    lines.append("\t\t\tprojectDirPath = \"\";")
    lines.append("\t\t\tprojectRoot = \"\";")
    lines.append("\t\t\ttargets = (")
    lines.append(f"\t\t\t\t{target} /* Zhunji */,")
    lines.append("\t\t\t);")
    lines.append("\t\t};")
    lines.append("/* End PBXProject section */")
    lines.append("")

    lines.append("/* Begin PBXResourcesBuildPhase section */")
    lines.append(f"\t\t{resources_phase} /* Resources */ = {{")
    lines.append("\t\t\tisa = PBXResourcesBuildPhase;")
    lines.append("\t\t\tbuildActionMask = 2147483647;")
    lines.append("\t\t\tfiles = (")
    for name in resources:
        lines.append(f"\t\t\t\t{res_build_files[name]} /* {name} in Resources */,")
    lines.append("\t\t\t);")
    lines.append("\t\t\trunOnlyForDeploymentPostprocessing = 0;")
    lines.append("\t\t};")
    lines.append("/* End PBXResourcesBuildPhase section */")
    lines.append("")

    lines.append("/* Begin PBXSourcesBuildPhase section */")
    lines.append(f"\t\t{sources_phase} /* Sources */ = {{")
    lines.append("\t\t\tisa = PBXSourcesBuildPhase;")
    lines.append("\t\t\tbuildActionMask = 2147483647;")
    lines.append("\t\t\tfiles = (")
    for uid_bf, _, comment in build_files:
        lines.append(f"\t\t\t\t{uid_bf} /* {comment} */,")
    lines.append("\t\t\t);")
    lines.append("\t\t\trunOnlyForDeploymentPostprocessing = 0;")
    lines.append("\t\t};")
    lines.append("/* End PBXSourcesBuildPhase section */")
    lines.append("")

    lines.append("/* Begin XCBuildConfiguration section */")
    settings_debug = """{
			isa = XCBuildConfiguration;
			buildSettings = {
				ALWAYS_SEARCH_USER_PATHS = NO;
				CLANG_ANALYZER_NONNULL = YES;
				CLANG_ENABLE_MODULES = YES;
				CLANG_ENABLE_OBJC_ARC = YES;
				CODE_SIGN_IDENTITY = "-";
				DEBUG_INFORMATION_FORMAT = dwarf;
				ENABLE_TESTABILITY = YES;
				GCC_NO_COMMON_BLOCKS = YES;
				MACOSX_DEPLOYMENT_TARGET = 14.0;
				ONLY_ACTIVE_ARCH = YES;
				SDKROOT = macosx;
				SWIFT_ACTIVE_COMPILATION_CONDITIONS = DEBUG;
				SWIFT_OPTIMIZATION_LEVEL = "-Onone";
				SWIFT_VERSION = 5.0;
			};
			name = Debug;
		};"""
    settings_release = """{
			isa = XCBuildConfiguration;
			buildSettings = {
				ALWAYS_SEARCH_USER_PATHS = NO;
				CLANG_ANALYZER_NONNULL = YES;
				CLANG_ENABLE_MODULES = YES;
				CLANG_ENABLE_OBJC_ARC = YES;
				CODE_SIGN_IDENTITY = "-";
				DEBUG_INFORMATION_FORMAT = "dwarf-with-dsym";
				GCC_NO_COMMON_BLOCKS = YES;
				MACOSX_DEPLOYMENT_TARGET = 14.0;
				SDKROOT = macosx;
				SWIFT_COMPILATION_MODE = wholemodule;
				SWIFT_OPTIMIZATION_LEVEL = "-O";
				SWIFT_VERSION = 5.0;
			};
			name = Release;
		};"""
    target_settings_debug = """{
			isa = XCBuildConfiguration;
			buildSettings = {
				ARCHS = arm64;
				CODE_SIGN_ENTITLEMENTS = Entitlements.plist;
				CODE_SIGN_STYLE = Automatic;
				CURRENT_PROJECT_VERSION = 1;
				ENABLE_HARDENED_RUNTIME = YES;
				INFOPLIST_FILE = Info.plist;
				LD_RUNPATH_SEARCH_PATHS = (
					"$(inherited)",
					"@executable_path/../Frameworks",
				);
				MARKETING_VERSION = 1.0;
				OTHER_LDFLAGS = (
					"$(inherited)",
					"-L$(SRCROOT)/Resources",
					"-lzhunji_core",
				);
				PRODUCT_BUNDLE_IDENTIFIER = com.zhunji.app;
				PRODUCT_NAME = "$(TARGET_NAME)";
				SWIFT_EMIT_LOC_STRINGS = NO;
			};
			name = Debug;
		};"""
    target_settings_release = target_settings_debug.replace(
        'name = Debug;', 'name = Release;'
    )
    lines.append(f"\t\t{cfg_debug_project} /* Debug */ = {settings_debug}")
    lines.append(f"\t\t{cfg_release_project} /* Release */ = {settings_release}")
    lines.append(f"\t\t{cfg_debug} /* Debug */ = {target_settings_debug}")
    lines.append(f"\t\t{cfg_release} /* Release */ = {target_settings_release}")
    lines.append("/* End XCBuildConfiguration section */")
    lines.append("")

    lines.append("/* Begin XCConfigurationList section */")
    lines.append(f"\t\t{project_cfglist} /* Build configuration list for PBXProject \"Zhunji\" */ = {{")
    lines.append("\t\t\tisa = XCConfigurationList;")
    lines.append("\t\t\tbuildConfigurations = (")
    lines.append(f"\t\t\t\t{cfg_debug_project} /* Debug */,")
    lines.append(f"\t\t\t\t{cfg_release_project} /* Release */,")
    lines.append("\t\t\t);")
    lines.append("\t\t\tdefaultConfigurationIsVisible = 0;")
    lines.append("\t\t\tdefaultConfigurationName = Release;")
    lines.append("\t\t};")
    lines.append(f"\t\t{target_cfglist} /* Build configuration list for PBXNativeTarget \"Zhunji\" */ = {{")
    lines.append("\t\t\tisa = XCConfigurationList;")
    lines.append("\t\t\tbuildConfigurations = (")
    lines.append(f"\t\t\t\t{cfg_debug} /* Debug */,")
    lines.append(f"\t\t\t\t{cfg_release} /* Release */,")
    lines.append("\t\t\t);")
    lines.append("\t\t\tdefaultConfigurationIsVisible = 0;")
    lines.append("\t\t\tdefaultConfigurationName = Release;")
    lines.append("\t\t};")
    lines.append("/* End XCConfigurationList section */")
    lines.append("")
    lines.append("\t};")
    lines.append(f"\trootObject = {project} /* Project object */;")
    lines.append("}")

    with open(os.path.join(PROJECT_DIR, "project.pbxproj"), "w") as f:
        f.write("\n".join(lines) + "\n")
    print(f"已生成 {os.path.join(PROJECT_DIR, 'project.pbxproj')}（{len(swift_files)} 个 Swift 文件）")


if __name__ == "__main__":
    main()
