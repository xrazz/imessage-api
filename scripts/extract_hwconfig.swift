import Foundation
import IOKit
import CommonCrypto

func mainPort() -> mach_port_t {
    if #available(macOS 12.0, *) { return kIOMainPortDefault }
    return kIOMasterPortDefault
}

func data(_ entry: io_registry_entry_t, _ key: String) -> Data {
    guard let value = IORegistryEntryCreateCFProperty(entry, key as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? Data else {
        fatalError("missing IORegistry data key: \(key)")
    }
    return value
}

func string(_ entry: io_registry_entry_t, _ key: String) -> String {
    guard let value = IORegistryEntryCreateCFProperty(entry, key as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? String else {
        fatalError("missing IORegistry string key: \(key)")
    }
    return value
}

func item(_ entry: io_registry_entry_t, _ key: String) -> String {
    String(data: data(entry, key), encoding: .utf8)!
        .trimmingCharacters(in: CharacterSet(charactersIn: "\0"))
}

func optionalItem(_ entry: io_registry_entry_t, _ key: String) -> String? {
    guard let value = IORegistryEntryCreateCFProperty(entry, key as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? Data else {
        return nil
    }
    return String(data: value, encoding: .utf8)?
        .trimmingCharacters(in: CharacterSet(charactersIn: "\0"))
}

func sysctlString(_ name: String) -> String {
    var size = 0
    sysctlbyname(name, nil, &size, nil, 0)
    var buffer = [CChar](repeating: 0, count: size)
    sysctlbyname(name, &buffer, &size, nil, 0)
    return String(cString: buffer)
}

func sha256(_ bytes: Data) -> Data {
    var digest = [UInt8](repeating: 0, count: Int(CC_SHA256_DIGEST_LENGTH))
    bytes.withUnsafeBytes { raw in
        _ = CC_SHA256(raw.baseAddress, CC_LONG(bytes.count), &digest)
    }
    return Data(digest)
}

func macAddress() -> Data {
    let filter = IOServiceMatching("IOEthernetInterface") as NSMutableDictionary
    filter["IOPropertyMatch"] = ["IOPrimaryInterface": true] as CFDictionary
    var iterator: io_iterator_t = 0
    IOServiceGetMatchingServices(mainPort(), filter, &iterator)
    let ethernet = IOIteratorNext(iterator)
    var parent: io_registry_entry_t = 0
    IORegistryEntryGetParentEntry(ethernet, kIOServicePlane, &parent)
    return data(parent, "IOMACAddress")
}

let deviceTree = IORegistryEntryFromPath(mainPort(), "IODeviceTree:/")
let ioPower = IORegistryEntryFromPath(mainPort(), "IOPower:/")
let optionsTree = IORegistryEntryFromPath(mainPort(), "IODeviceTree:/options")
let chosenTree = IORegistryEntryFromPath(mainPort(), "IODeviceTree:/chosen")

let rom: Data
if let value = IORegistryEntryCreateCFProperty(optionsTree, "4D1EDE05-38C7-4A6A-9CC6-4BCCA8B38C14:ROM" as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? Data {
    rom = value
} else {
    rom = sha256(data(chosenTree, "unique-chip-id")).suffix(6)
}

let boardID: String
if let value = IORegistryEntryCreateCFProperty(deviceTree, "board-id" as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? Data,
   let decoded = String(data: value, encoding: .utf8) {
    boardID = decoded.trimmingCharacters(in: CharacterSet(charactersIn: "\0"))
} else {
    boardID = "Mac-" + data(chosenTree, "board-id").map { String(format: "%02hhx", $0) }.joined()
}

let mlb: String
if let value = IORegistryEntryCreateCFProperty(optionsTree, "4D1EDE05-38C7-4A6A-9CC6-4BCCA8B38C14:MLB" as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? Data,
   let decoded = String(data: value, encoding: .utf8) {
    mlb = decoded.trimmingCharacters(in: CharacterSet(charactersIn: "\0"))
} else {
    mlb = item(deviceTree, "mlb-serial-number")
}

let inner: [String: Any] = [
    "product_name": optionalItem(deviceTree, "product-name") ?? item(deviceTree, "model"),
    "io_mac_address": macAddress(),
    "platform_serial_number": string(deviceTree, "IOPlatformSerialNumber"),
    "platform_uuid": string(deviceTree, "IOPlatformUUID"),
    "root_disk_uuid": item(chosenTree, "boot-uuid"),
    "board_id": boardID,
    "os_build_num": sysctlString("kern.osversion"),
    "platform_serial_number_enc": data(ioPower, "Gq3489ugfi"),
    "platform_uuid_enc": data(ioPower, "Fyp98tpgj"),
    "root_disk_uuid_enc": data(ioPower, "kbjfrfpoJU"),
    "rom": rom,
    "rom_enc": data(ioPower, "oycqAZloTNDm"),
    "mlb": mlb,
    "mlb_enc": data(ioPower, "abKPld1EcMni"),
]

let plist: [String: Any] = [
    "inner": inner,
    "version": sysctlString("kern.osproductversion"),
    "protocol_version": 1660,
    "device_id": UUID().uuidString,
    "icloud_ua": "com.apple.iCloudHelper/282 CFNetwork/1408.0.4 Darwin/22.5.0",
    "aoskit_version": "com.apple.AOSKit/282 (com.apple.accountsd/113)",
    "udid": "55A1CFBF5BB56AD1159BD2CB7D6FF546E48EAAE4BF16188A07B1FB9C83138CA2",
]

let output = try PropertyListSerialization.data(
    fromPropertyList: plist,
    format: .xml,
    options: 0
)
FileHandle.standardOutput.write(output)
