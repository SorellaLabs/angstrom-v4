// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.24;

import {IPoolManager} from "v4-core/src/interfaces/IPoolManager.sol";
import {PoolKey} from "v4-core/src/types/PoolKey.sol";
import {SwapParams} from "v4-core/src/types/PoolOperation.sol";
import {BalanceDelta} from "v4-core/src/types/BalanceDelta.sol";
import {IUnlockCallback} from "v4-core/src/interfaces/callback/IUnlockCallback.sol";

contract SwapQuoter is IUnlockCallback {
    IPoolManager public immutable manager;

    error QuoteResult(int128 amount0, int128 amount1);
    error UnexpectedRevert(bytes reason);

    constructor(IPoolManager _manager) {
        manager = _manager;
    }

    function quote(
        PoolKey memory key,
        SwapParams memory params,
        bytes memory hookData
    ) external returns (int128 amount0, int128 amount1) {
        try manager.unlock(abi.encode(key, params, hookData)) {
            revert("Expected revert");
        } catch (bytes memory reason) {
            // Verify the revert is our QuoteResult error before decoding
            bytes4 selector;
            assembly {
                selector := mload(add(reason, 32))
            }
            if (selector != QuoteResult.selector || reason.length < 68) {
                revert UnexpectedRevert(reason);
            }
            assembly {
                amount0 := mload(add(reason, 36))
                amount1 := mload(add(reason, 68))
            }
        }
    }

    function unlockCallback(bytes calldata data) external returns (bytes memory) {
        require(msg.sender == address(manager));
        (PoolKey memory key, SwapParams memory params, bytes memory hookData) =
            abi.decode(data, (PoolKey, SwapParams, bytes));
        BalanceDelta delta = manager.swap(key, params, hookData);
        revert QuoteResult(delta.amount0(), delta.amount1());
    }
}
