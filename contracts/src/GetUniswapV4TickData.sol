// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import {IPoolManager} from "v4-core/src/interfaces/IPoolManager.sol";
import {TickMath} from "v4-core/src/libraries/TickMath.sol";
import {PoolId} from "v4-core/src/types/PoolId.sol";
import {IUniV4} from "core/src/interfaces/IUniV4.sol";

contract GetUniswapV4TickData {
    struct TickData {
        bool initialized;
        int24 tick;
        uint128 liquidityGross;
        int128 liquidityNet;
    }

    struct TicksWithBlock {
        TickData[] ticks;
        uint256 validTo;
        uint256 blockNumber;
    }

    constructor(
        PoolId poolId,
        address poolManager,
        bool zeroForOne,
        int24 currentTick,
        uint16 numTicks,
        int24 tickSpacing
    ) {
        TickData[] memory tickData = new TickData[](numTicks);

        //Instantiate current word position to keep track of the word count
        uint256 counter = 0;

        while (counter < numTicks) {
            (bool initialized, int24 nextTick) = zeroForOne
                ? IUniV4.getNextTickLe(
                    IPoolManager(poolManager),
                    poolId,
                    currentTick,
                    tickSpacing
                )
                : IUniV4.getNextTickGt(
                    IPoolManager(poolManager),
                    poolId,
                    currentTick,
                    tickSpacing
                );

            (uint128 liquidityGross, int128 liquidityNet) = IUniV4
                .getTickLiquidity(IPoolManager(poolManager), poolId, nextTick);

            //Make sure not to overshoot the max/min tick
            //If we do, break the loop, and set the last initialized tick to the max/min tick=
            if (nextTick < TickMath.MIN_TICK || nextTick >= TickMath.MAX_TICK) {
                break;
            } else {
                tickData[counter].initialized = initialized;
                tickData[counter].tick = nextTick;
                tickData[counter].liquidityGross = liquidityGross;
                tickData[counter].liquidityNet = liquidityNet;
            }

            counter++;

            currentTick = nextTick;
            if (zeroForOne) {
                --currentTick;
            }
        }

        TicksWithBlock memory ticksWithBlock = TicksWithBlock({
            ticks: tickData,
            validTo: counter,
            blockNumber: block.number
        });

        // ensure abi encoding, not needed here but increase reusability for different return types
        // note: abi.encode add a first 32 bytes word with the address of the original data
        bytes memory abiEncodedData = abi.encode(ticksWithBlock);

        assembly {
            let dataStart := add(abiEncodedData, 0x20)
            let dataSize := mload(abiEncodedData)
            return(dataStart, dataSize)
        }
    }
}
