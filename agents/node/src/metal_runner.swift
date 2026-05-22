import Foundation
import Metal

struct WorkerResponse: Codable {
    let job_id: String
    let worker_id: String
    let status: String
    let output: String
    let error: String?
    let backend: String
    let node_id: String
}

func argumentValue(_ flag: String) -> String? {
    let args = CommandLine.arguments
    for index in 0..<args.count {
        if args[index] == flag, index + 1 < args.count {
            return args[index + 1]
        }
    }
    return nil
}

func emit(_ response: WorkerResponse) {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    if let data = try? encoder.encode(response),
       let text = String(data: data, encoding: .utf8) {
        print(text)
    } else {
        print("""
        {"job_id":"\(response.job_id)","worker_id":"\(response.worker_id)","status":"failed","output":"","error":"unable to serialize response","backend":"\(response.backend)","node_id":"\(response.node_id)"}
        """)
    }
}

let jobId = argumentValue("--job-id") ?? "job-unknown"
let nodeId = argumentValue("--node-id") ?? "node-unknown"
let prompt = argumentValue("--prompt") ?? ""
let model = argumentValue("--model") ?? "default"
let workerId = "worker-" + UUID().uuidString.replacingOccurrences(of: "-", with: "").prefix(12)
let workerIdString = String(workerId)

guard let device = MTLCreateSystemDefaultDevice() else {
    emit(WorkerResponse(
        job_id: jobId,
        worker_id: workerIdString,
        status: "failed",
        output: "",
        error: "Metal device unavailable",
        backend: "m",
        node_id: nodeId
    ))
    exit(0)
}

guard let queue = device.makeCommandQueue() else {
    emit(WorkerResponse(
        job_id: jobId,
        worker_id: workerIdString,
        status: "failed",
        output: "",
        error: "Metal command queue unavailable",
        backend: "m",
        node_id: nodeId
    ))
    exit(0)
}

let source = """
#include <metal_stdlib>
using namespace metal;

kernel void opengpu_fill(device uint *buffer [[buffer(0)]],
                         uint id [[thread_position_in_grid]]) {
    buffer[id] = (id * 17u) ^ (id + 11u);
}
"""

do {
    let library = try device.makeLibrary(source: source, options: nil)
    guard let function = library.makeFunction(name: "opengpu_fill") else {
        throw NSError(domain: "OpenGPU", code: 1, userInfo: [NSLocalizedDescriptionKey: "missing Metal kernel"])
    }

    let pipeline = try device.makeComputePipelineState(function: function)
    let tokenCount = prompt.split { $0.isWhitespace }.filter { !$0.isEmpty }.count
    let elements = max(64, min(4096, max(1, tokenCount) * 32))
    let byteCount = elements * MemoryLayout<UInt32>.stride
    guard let buffer = device.makeBuffer(length: byteCount, options: .storageModeShared) else {
        throw NSError(domain: "OpenGPU", code: 2, userInfo: [NSLocalizedDescriptionKey: "failed to allocate Metal buffer"])
    }

    let bufferPointer = buffer.contents().bindMemory(to: UInt32.self, capacity: elements)
    let modelSeed = UInt32(model.count & 0xff)
    let promptSeed = UInt32(prompt.count & 0xff)
    for index in 0..<elements {
        bufferPointer[index] = UInt32(index) &+ modelSeed &+ promptSeed
    }

    guard let commandBuffer = queue.makeCommandBuffer(),
          let encoder = commandBuffer.makeComputeCommandEncoder() else {
        throw NSError(domain: "OpenGPU", code: 3, userInfo: [NSLocalizedDescriptionKey: "failed to create Metal command buffer"])
    }

    encoder.setComputePipelineState(pipeline)
    encoder.setBuffer(buffer, offset: 0, index: 0)

    let threadgroupWidth = max(1, min(pipeline.threadExecutionWidth, elements))
    let threadsPerThreadgroup = MTLSize(width: threadgroupWidth, height: 1, depth: 1)
    let threadgroups = MTLSize(
        width: (elements + threadgroupWidth - 1) / threadgroupWidth,
        height: 1,
        depth: 1
    )

    encoder.dispatchThreadgroups(threadgroups, threadsPerThreadgroup: threadsPerThreadgroup)
    encoder.endEncoding()
    commandBuffer.commit()
    commandBuffer.waitUntilCompleted()

    let values = UnsafeBufferPointer(start: bufferPointer, count: elements)
    let checksum = values.reduce(0 as UInt64) { partial, value in
        (partial &* 131) &+ UInt64(value)
    }
    let sample = values.prefix(8).map(String.init).joined(separator: ",")
    let output = "metal=device:\(device.name); elements=\(elements); checksum=\(checksum); sample=\(sample); model=\(model)"

    emit(WorkerResponse(
        job_id: jobId,
        worker_id: workerIdString,
        status: "completed",
        output: output,
        error: nil,
        backend: "m",
        node_id: nodeId
    ))
} catch {
    emit(WorkerResponse(
        job_id: jobId,
        worker_id: workerIdString,
        status: "failed",
        output: "",
        error: error.localizedDescription,
        backend: "m",
        node_id: nodeId
    ))
}
