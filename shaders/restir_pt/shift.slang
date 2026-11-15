/***************************************************************************
 # Copyright (c) 2015-26, NVIDIA CORPORATION. All rights reserved.
 #
 # Redistribution and use in source and binary forms, with or without
 # modification, are permitted provided that the following conditions
 # are met:
 #  * Redistributions of source code must retain the above copyright
 #    notice, this list of conditions and the following disclaimer.
 #  * Redistributions in binary form must reproduce the above copyright
 #    notice, this list of conditions and the following disclaimer in the
 #    documentation and/or other materials provided with the distribution.
 #  * Neither the name of NVIDIA CORPORATION nor the names of its
 #    contributors may be used to endorse or promote products derived
 #    from this software without specific prior written permission.
 #
 # THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS "AS IS" AND ANY
 # EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 # IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
 # PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR
 # CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
 # EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
 # PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
 # PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY
 # OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 # (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
 # OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 **************************************************************************/

/// Falcor 759aad03 默认 point Hybrid shift；原始阈值、双向可逆域与 mixed-PSS Jacobian。
/// 不持有绑定或世界状态；sample/eval/trace 由场景的独立窄入口执行。
module shift;

import restir_pt.types;
import restir_pt.scene;
import restir_pt.replay;
import restir_pt.storage;

namespace restir_pt
{
func invalidShiftJacobian(jacobian: float)->bool
{
    return jacobian <= 0.0 || isnan(jacobian) || isinf(jacobian);
}

func invalidShiftIntegrand(integrand: float3)->bool
{
    return all(integrand == 0.0) || any(isnan(integrand) | isinf(integrand) | (integrand < 0.0));
}

func rejectReconnection(inout jacobian: float, inout sourceGeometry: float,
                        destinationGeometry: float)
    ->float3
{
    jacobian = 0.0;
    sourceGeometry = destinationGeometry;
    return 0.0;
}

/// 只执行 reconnection 两侧必要的连续 BSDF 求值与连接可见性，不采样新路径。
/// source.reconnectionJacobian 更新为目标 geometry factor，供后续 GRIS 链接。
/// 默认 additionalShadowRayOffset=false；受控端点偏移由 scene 的精确 visibility 维护。
public func shiftPathReconnection<Scene : IRestirScene>(inout scene: Scene, settings: ShiftSettings,
                                                        primaryRayFootprint: float,
                                                        rcFootprintFactor: float,
                                                        inout jacobian: float, dstPrevious: Surface,
                                                        inout source: PathReservoir, useNEE: bool,
                                                        useMIS: bool, previous: bool)
    ->float3
{
    let rcLength : uint = source.pathFlags.rcVertexLength();
    let pathLength : uint = source.pathFlags.pathLength();
    if (source.pathFlags.decodeIsDeltaEvent(true) || source.pathFlags.decodeIsDeltaEvent(false))
        return 0.0;
    if (dstPrevious.supportBSDFIndexing !=
        source.pathFlags.decodeSupportBSDFIndexingBeforeRcVertex())
        return 0.0;

    let rcHasPosition : bool = source.reconnectionHit.isValid();
    let rcIsConnectedToLight : bool = pathLength == rcLength;
    let rcIsEmissive : bool = pathLength + 1 == rcLength;
    let lightIsNEE : bool = source.pathFlags.lastVertexNEE();
    let rcIsNEE : bool = rcIsConnectedToLight && lightIsNEE;
    let lightType : uint = source.pathFlags.lightType();
    var rcSurface : Surface = {};
    var direction : float3 = source.reconnectionWi;
    var destinationGeometry = 1.0;
    var inverseGeometry = 1.0;
    var pointLight = false;
    if (rcHasPosition)
    {
        rcSurface = scene.loadSurfaceFromOrigin(source.reconnectionHit, dstPrevious.point.position,
                                                previous);
        if (!rcSurface.hit.isValid())
            return rejectReconnection(jacobian, source.reconnectionJacobian, 0.0);
        let delta : float3 = rcSurface.point.position - dstPrevious.point.position;
        let distanceSquared : float = dot(delta, delta);
        direction = delta * rsqrt(distanceSquared);
        pointLight = lightType == LIGHT_ANALYTIC && rcIsEmissive &&
                     scene.isPointLight(source.reconnectionHit);
        destinationGeometry =
            (pointLight ? 1.0 : abs(dot(rcSurface.point.normal, -direction))) / distanceSquared;
        inverseGeometry = distanceSquared / abs(dot(dstPrevious.point.normal, -direction));
    }
    // 这些拒绝仍须保留最终 G；上方 delta/indexing 拒绝沿用原有不写回语义。
    if (rcHasPosition && rcIsEmissive && !pointLight && !scene.emissionSideSupported(rcSurface))
        return rejectReconnection(jacobian, source.reconnectionJacobian, destinationGeometry);

    var distanceThreshold : float;
    var roughnessThreshold : float;
    initReplaySampler(source.initRandomSeed, settings, distanceThreshold, roughnessThreshold);
    let firstComponent : int = dstPrevious.supportBSDFIndexing && !(rcIsEmissive && lightIsNEE)
                                   ? int(source.pathFlags.decodeBSDFComponent(true))
                                   : -1;
    let connection
        : ConnectionEvaluation = scene.evaluateConnection(
              dstPrevious, direction,
              rcHasPosition ? rcSurface.distance : scene.environmentDistance(), firstComponent);
    let first : BsdfEvaluation = connection.bsdf;
    // rcPrevious 在目标前缀中不能已成为更早的 reconnection vertex。
    if (rcLength > 1 && rcFootprintFactor > 0.0 &&
        rcFootprintFactor / first.pdf >= primaryRayFootprint * distanceThreshold)
        return rejectReconnection(jacobian, source.reconnectionJacobian, destinationGeometry);
    let checkReverseThreshold : bool = settings.shiftMapping == 1 && !(rcIsEmissive && lightIsNEE);
    if (checkReverseThreshold)
    {
        let violatesRoughness
            : bool =
                  !(rcIsEmissive && !lightIsNEE && !dstPrevious.supportSimpleRoughnessThreshold) &&
                  !scene.isSampleRough(dstPrevious, source.pathFlags.decodeBSDFComponent(true),
                                       first.pdf, roughnessThreshold);
        let rayFootprint : float = 1.0 / (first.pdf * destinationGeometry);
        let violatesDistance
            : bool = rcHasPosition && rayFootprint < primaryRayFootprint * distanceThreshold;
        if (violatesRoughness || violatesDistance)
            return rejectReconnection(jacobian, source.reconnectionJacobian, destinationGeometry);
    }
    let incidentRadiance
        : float3 = rcIsEmissive && !rcHasPosition && lightType == LIGHT_ENV
                       ? scene.environment(direction, dstPrevious.point.position, previous)
                       : source.reconnectionIncidentRadiance;
    var integrand : float3 = incidentRadiance * first.value;
    integrand *= connection.transmittance;
    if (rcIsEmissive && useNEE && useMIS && lightType != LIGHT_ANALYTIC &&
        !(lightIsNEE && source.pathFlags.terminalNEE()))
    {
        var lightPdf : float = source.lightPdf / destinationGeometry;
        if ((rcHasPosition && lightType == LIGHT_EMISSIVE) ||
            (!rcHasPosition && lightType == LIGHT_ENV))
            lightPdf =
                scene.lightPdf(dstPrevious, source.reconnectionHit, direction, lightType, previous);
        integrand *= lightIsNEE ? lightMisWeight(1.0, lightPdf, 1.0, first.pdf)
                                : lightMisWeight(1.0, first.pdf, 1.0, lightPdf);
    }

    let needSecond : bool = rcHasPosition && !rcIsEmissive;
    var secondPdf : float = 0.5 / 3.14159265358979323846;
    if (needSecond)
    {
        rcSurface.incident.x = connection.incidentIor;
        let secondComponent : int = rcSurface.supportBSDFIndexing && !rcIsNEE
                                        ? int(source.pathFlags.decodeBSDFComponent(false))
                                        : -1;
        let second : BsdfEvaluation =
                         scene.evaluateBsdf(rcSurface, source.reconnectionWi, secondComponent);
        integrand *= second.value;
        secondPdf = second.pdf;
        if (rcIsConnectedToLight && useNEE && useMIS && lightType != LIGHT_ANALYTIC &&
            !(rcIsNEE && source.pathFlags.terminalNEE()))
            integrand *= rcIsNEE ? lightMisWeight(1.0, source.lightPdf, 1.0, secondPdf)
                                 : lightMisWeight(1.0, secondPdf, 1.0, source.lightPdf);
    }
    // emission-BSDF 端点使用 NV 的 diffuse pseudo PDF；NEE 强制光源重连不做反向阈值。
    if (checkReverseThreshold && !(rcIsEmissive && !rcHasPosition) &&
        inverseGeometry / secondPdf < primaryRayFootprint * distanceThreshold)
        return rejectReconnection(jacobian, source.reconnectionJacobian, destinationGeometry);
    jacobian *= destinationGeometry / source.reconnectionJacobian;
    // 负值只在所有 BSDF/MIS 乘积完成后判定，避免提前拒绝随后会被零消去的分量。
    if (invalidShiftJacobian(jacobian) || invalidShiftIntegrand(integrand))
        return rejectReconnection(jacobian, source.reconnectionJacobian, destinationGeometry);
    let visible : bool = rcHasPosition ? scene.visible(dstPrevious, rcSurface, previous)
                                       : scene.visibleDirection(dstPrevious, direction, previous);
    if (!visible)
        return rejectReconnection(jacobian, source.reconnectionJacobian, destinationGeometry);
    source.reconnectionJacobian = destinationGeometry;
    return integrand;
}

/// 使用已完成的 prefix 重放结果；空 rcPrevHit 只允许无 reconnection 的完整 PSS 路径。
public func shiftPathFromReplay<Scene : IRestirScene>(inout scene: Scene, settings: ShiftSettings,
                                                      primaryRayFootprint: float,
                                                      replay: ReconnectionData,
                                                      inout source: PathReservoir, useNEE: bool,
                                                      useMIS: bool, previous: bool,
                                                      out jacobian: float)
    ->float3
{
    jacobian = 1.0;
    var throughput : float3 = source.weight != 0.0 ? replay.pathThroughput : float3(0.0);
    if (any(throughput > 0.0) && replay.rcPrevHit.isValid())
    {
        var previousSurface : Surface =
                                  scene.loadSurface(replay.rcPrevHit, replay.rcPrevWo, previous);
        if (!previousSurface.hit.isValid())
        {
            jacobian = 0.0;
            return 0.0;
        }
        scene.setIncident(previousSurface, replay.rcPrevIncident);
        throughput *=
            shiftPathReconnection(scene, settings, primaryRayFootprint, replay.rcFootprintFactor,
                                  jacobian, previousSurface, source, useNEE, useMIS, previous);
    }
    else if (source.pathFlags.rcVertexLength() <= source.pathFlags.pathLength() + 1)
    {
        throughput = 0.0;
    }
    return throughput;
}

/// 只读源在本轮保持不变；prefix只载入12B热字段，冷身份在其结束后恢复。
/// 这样replay中的材质callable/SER无需保存整份80B源；所有端点/介质仍由场景精确恢复。
/// 两源入口只在加载边界选择A/B，replay/shift体保持单份，避免temporal方向复制整个RT控制流。
/// historySource只选源buffer；previous只选目标几何/光照世代，二者语义独立。
/// buffer与index引用固定本轮A或已接受历史B，不在本函数提交、交换或修改源。
public func shiftPathLoaded<Scene : IRestirScene>(inout scene: Scene, primary: Surface,
                                                  current: RWByteAddressBuffer,
                                                  history: RWByteAddressBuffer, index: uint,
                                                  historySource: bool, primaryRayFootprint: float,
                                                  settings: ShiftSettings, useRussianRoulette: bool,
                                                  useNEE: bool, useMIS: bool, previous: bool)
    ->PrecomputedShiftData
{
    var source : PathReservoir = historySource ? load_replay_source(history, index)
                                               : load_replay_source(current, index);
    let replay : ReconnectionData =
                     traceReplayedPath(scene, primary, source, primaryRayFootprint, settings,
                                       useRussianRoulette, useNEE, useMIS, previous);
    source = historySource ? load_reservoir(history, index) : load_reservoir(current, index);
    var result : PrecomputedShiftData = {};
    result.pathThroughput = shiftPathFromReplay(scene, settings, primaryRayFootprint, replay,
                                                source, useNEE, useMIS, previous, result.Jacobian);
    result.reconnectionJacobian = source.reconnectionJacobian;
    return result;
}

/// 单一只读源的入口；复用同一冷热恢复契约，编译器消除恒定false的历史选择。
public func shiftPathLoaded<Scene : IRestirScene>(inout scene: Scene, primary: Surface,
                                                  buffer: RWByteAddressBuffer, index: uint,
                                                  primaryRayFootprint: float,
                                                  settings: ShiftSettings, useRussianRoulette: bool,
                                                  useNEE: bool, useMIS: bool, previous: bool)
    ->PrecomputedShiftData
{
    return shiftPathLoaded(scene, primary, buffer, buffer, index, false, primaryRayFootprint,
                           settings, useRussianRoulette, useNEE, useMIS, previous);
}

/// 完整的 point shift；逆向 pairwise MIS 必须独立以反向 primary 再调用一次，不能反用已改写的
/// source。
public func shiftPath<Scene : IRestirScene>(inout scene: Scene, primary: Surface,
                                            inout source: PathReservoir, primaryRayFootprint: float,
                                            settings: ShiftSettings, useRussianRoulette: bool,
                                            useNEE: bool, useMIS: bool, previous: bool,
                                            out jacobian: float)
    ->float3
{
    let replay : ReconnectionData =
                     traceReplayedPath(scene, primary, source, primaryRayFootprint, settings,
                                       useRussianRoulette, useNEE, useMIS, previous);
    return shiftPathFromReplay(scene, settings, primaryRayFootprint, replay, source, useNEE, useMIS,
                               previous, jacobian);
}
}
