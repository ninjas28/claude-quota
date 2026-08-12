// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "ClaudeQuotaBar",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "ClaudeQuotaBar", targets: ["ClaudeQuotaBar"])
    ],
    targets: [
        .executableTarget(
            name: "ClaudeQuotaBar",
            path: "Sources/ClaudeQuotaBar"
        ),
        .testTarget(
            name: "ClaudeQuotaBarTests",
            dependencies: ["ClaudeQuotaBar"],
            path: "Tests/ClaudeQuotaBarTests"
        )
    ]
)
