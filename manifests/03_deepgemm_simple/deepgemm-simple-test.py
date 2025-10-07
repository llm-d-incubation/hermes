#!/usr/bin/env python3

# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
# http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

import argparse
import sys
import traceback
import torch
import random


def test_basic_fp8_gemm():
    """Test basic FP8 GEMM functionality."""
    print("Testing basic FP8 GEMM...")

    try:
        import deep_gemm
        from deep_gemm.testing import calc_diff

        # Simple test case
        m, n, k = 256, 256, 256

        # Create FP8 tensors (E4M3 format)
        a_fp32 = torch.randn(m, k, device='cuda', dtype=torch.float32)
        b_fp32 = torch.randn(k, n, device='cuda', dtype=torch.float32)

        # Convert to FP8
        a_fp8 = a_fp32.to(torch.float8_e4m3fn)
        b_fp8 = b_fp32.to(torch.float8_e4m3fn)
        a_scale = torch.tensor(1.0, device='cuda', dtype=torch.float32)
        b_scale = torch.tensor(1.0, device='cuda', dtype=torch.float32)

        # Output tensor
        c = torch.zeros(m, n, device='cuda', dtype=torch.bfloat16)

        # Run DeepGEMM
        deep_gemm.fp8_gemm_nt((a_fp8, a_scale), (b_fp8, b_scale), c)

        # Reference computation
        ref = torch.mm(a_fp32.to(torch.bfloat16), b_fp32.to(torch.bfloat16).T)

        # Check difference
        diff = calc_diff(c, ref)
        print(f"  FP8 GEMM diff: {diff:.6f}")

        if diff < 0.1:  # More relaxed tolerance for FP8
            print("  ✅ FP8 GEMM test PASSED")
            return True
        else:
            print(f"  ❌ FP8 GEMM test FAILED (diff={diff})")
            return False

    except Exception as e:
        print(f"  ❌ FP8 GEMM test FAILED: {e}")
        traceback.print_exc()
        return False


def test_m_grouped_fp8_gemm():
    """Test M-grouped FP8 GEMM functionality."""
    print("Testing M-grouped FP8 GEMM...")

    try:
        import deep_gemm
        from deep_gemm.testing import calc_diff

        # Test parameters
        num_groups = 4
        m_per_group = 64
        n, k = 128, 128
        total_m = num_groups * m_per_group

        # Create input tensors
        a_fp32 = torch.randn(total_m, k, device='cuda', dtype=torch.float32)
        b_fp32 = [torch.randn(k, n, device='cuda', dtype=torch.float32) for _ in range(num_groups)]

        # Convert to FP8
        a_fp8 = a_fp32.to(torch.float8_e4m3fn)
        b_fp8 = [b.to(torch.float8_e4m3fn) for b in b_fp32]
        a_scale = torch.tensor(1.0, device='cuda', dtype=torch.float32)
        b_scales = [torch.tensor(1.0, device='cuda', dtype=torch.float32) for _ in range(num_groups)]

        # M indices for grouping
        m_indices = torch.arange(total_m, device='cuda', dtype=torch.int32)

        # Output tensor
        c = torch.zeros(total_m, n, device='cuda', dtype=torch.bfloat16)

        # Run DeepGEMM grouped operation
        deep_gemm.m_grouped_fp8_gemm_nt_contiguous(
            (a_fp8, a_scale),
            [(b, s) for b, s in zip(b_fp8, b_scales)],
            c,
            m_indices
        )

        # Reference computation
        ref = torch.zeros_like(c)
        for i in range(num_groups):
            start_idx = i * m_per_group
            end_idx = (i + 1) * m_per_group
            ref[start_idx:end_idx] = torch.mm(
                a_fp32[start_idx:end_idx].to(torch.bfloat16),
                b_fp32[i].to(torch.bfloat16).T
            )

        # Check difference
        diff = calc_diff(c, ref)
        print(f"  M-grouped FP8 GEMM diff: {diff:.6f}")

        if diff < 0.1:
            print("  ✅ M-grouped FP8 GEMM test PASSED")
            return True
        else:
            print(f"  ❌ M-grouped FP8 GEMM test FAILED (diff={diff})")
            return False

    except Exception as e:
        print(f"  ❌ M-grouped FP8 GEMM test FAILED: {e}")
        traceback.print_exc()
        return False


def test_library_import():
    """Test that DeepGEMM library can be imported and basic functions exist."""
    print("Testing DeepGEMM library import...")

    try:
        import deep_gemm
        import deep_gemm.testing

        # Check key functions exist
        required_functions = [
            'fp8_gemm_nt',
            'm_grouped_fp8_gemm_nt_contiguous',
            'get_num_sms',
            'get_tc_util'
        ]

        missing_functions = []
        for func_name in required_functions:
            if not hasattr(deep_gemm, func_name):
                missing_functions.append(func_name)

        if missing_functions:
            print(f"  ❌ Missing functions: {missing_functions}")
            return False

        # Check testing functions
        testing_functions = ['calc_diff', 'count_bytes', 'bench_kineto']
        missing_testing = []
        for func_name in testing_functions:
            if not hasattr(deep_gemm.testing, func_name):
                missing_testing.append(func_name)

        if missing_testing:
            print(f"  ❌ Missing testing functions: {missing_testing}")
            return False

        print("  ✅ Library import test PASSED")
        return True

    except Exception as e:
        print(f"  ❌ Library import test FAILED: {e}")
        traceback.print_exc()
        return False


def main():
    parser = argparse.ArgumentParser(description="Run simplified DeepGEMM self tests")
    parser.add_argument("--gpu", type=int, default=0, help="GPU device to use")
    args = parser.parse_args()

    # Set CUDA device
    if torch.cuda.is_available():
        torch.cuda.set_device(args.gpu)
        print(f"Using GPU {args.gpu}: {torch.cuda.get_device_name(args.gpu)}")
    else:
        print("❌ CUDA not available")
        sys.exit(1)

    print("DeepGEMM Simple Self Test")
    print(f"Device: {torch.cuda.get_device_name()}")
    print(f"CUDA Version: {torch.version.cuda}")
    print(f"PyTorch Version: {torch.__version__}")

    # Set up environment
    torch.manual_seed(42)
    random.seed(42)
    torch.backends.cuda.matmul.allow_tf32 = True
    torch.backends.cudnn.allow_tf32 = True

    # Run tests
    print(f"\n{'='*60}")
    print("RUNNING TESTS")
    print(f"{'='*60}")

    tests = [
        ("Library Import", test_library_import),
        ("Basic FP8 GEMM", test_basic_fp8_gemm),
        ("M-grouped FP8 GEMM", test_m_grouped_fp8_gemm),
    ]

    results = {}
    for test_name, test_func in tests:
        print(f"\n{test_name}:")
        results[test_name] = test_func()

    # Print summary
    print(f"\n{'='*60}")
    print("TEST SUMMARY")
    print(f"{'='*60}")

    total_tests = len(results)
    passed_tests = sum(results.values())
    failed_tests = total_tests - passed_tests

    for test_name, passed in results.items():
        status = "✅ PASSED" if passed else "❌ FAILED"
        print(f"{test_name:20} - {status}")

    print(f"\nTotal: {total_tests}, Passed: {passed_tests}, Failed: {failed_tests}")

    if failed_tests > 0:
        print(f"\n❌ {failed_tests} test(s) failed")
        sys.exit(1)
    else:
        print(f"\n✅ All tests passed!")
        sys.exit(0)


if __name__ == "__main__":
    main()
