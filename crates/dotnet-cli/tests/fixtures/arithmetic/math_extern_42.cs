using System;
using System.Diagnostics;
using System.Reflection;

public static class Program
{
    private static bool NegativeZero(double value) =>
        BitConverter.DoubleToInt64Bits(value) == long.MinValue;

    private static bool NegativeZero(float value) =>
        BitConverter.SingleToInt32Bits(value) == int.MinValue;

    private static bool CheckDoubleModF()
    {
        unsafe
        {
            double integral = 0;
            var method = typeof(Math).GetMethod("ModF", BindingFlags.NonPublic | BindingFlags.Static);
            if (method is null)
                return false;

            var fraction = (double)method.Invoke(null, new object?[] { -3.25, (IntPtr)(&integral) })!;
            if (fraction != -0.25 || integral != -3.0) return false;
            fraction = (double)method.Invoke(null, new object?[] { -3.0, (IntPtr)(&integral) })!;
            if (!NegativeZero(fraction) || integral != -3.0) return false;
            fraction = (double)method.Invoke(null, new object?[] { -0.0, (IntPtr)(&integral) })!;
            return NegativeZero(fraction) && NegativeZero(integral);
        }
    }

    private static bool CheckSingleModF()
    {
        unsafe
        {
            float integral = 0;
            var method = typeof(MathF).GetMethod("ModF", BindingFlags.NonPublic | BindingFlags.Static);
            if (method is null)
                return false;

            var fraction = (float)method.Invoke(null, new object?[] { -3.25f, (IntPtr)(&integral) })!;
            if (fraction != -0.25f || integral != -3.0f) return false;
            fraction = (float)method.Invoke(null, new object?[] { -3.0f, (IntPtr)(&integral) })!;
            if (!NegativeZero(fraction) || integral != -3.0f) return false;
            fraction = (float)method.Invoke(null, new object?[] { -0.0f, (IntPtr)(&integral) })!;
            return NegativeZero(fraction) && NegativeZero(integral);
        }
    }

    private static bool CheckDouble()
    {
        if (Math.Acos(1.0) != 0.0 || Math.Acosh(1.0) != 0.0) return false;
        if (Math.Asin(0.0) != 0.0 || Math.Asinh(0.0) != 0.0) return false;
        if (Math.Atan(0.0) != 0.0 || Math.Atanh(0.0) != 0.0) return false;
        if (Math.Atan2(0.0, 1.0) != 0.0 || Math.Cbrt(1.0) != 1.0) return false;
        if (Math.Ceiling(1.25) != 2.0 || Math.Ceiling(-1.25) != -1.0) return false;
        if (Math.Cos(0.0) != 1.0 || Math.Cosh(0.0) != 1.0 || Math.Exp(0.0) != 1.0) return false;
        if (Math.Floor(1.75) != 1.0 || Math.Floor(-1.75) != -2.0) return false;
        if (Math.FusedMultiplyAdd(2.0, 4.0, 1.0) != 9.0) return false;
        const double epsilon = 1.0 / 134217728.0;
        if (Math.FusedMultiplyAdd(1.0 + epsilon, 1.0 - epsilon, -1.0) != -1.0 / 18014398509481984.0) return false;
        if (Math.Log(1.0) != 0.0 || Math.Log2(1.0) != 0.0 || Math.Log10(1.0) != 0.0) return false;
        if (Math.Pow(2.0, 10.0) != 1024.0 || Math.Sin(0.0) != 0.0 || Math.Sinh(0.0) != 0.0) return false;
        if (Math.Sqrt(1.0) != 1.0 || Math.Tan(0.0) != 0.0 || Math.Tanh(0.0) != 0.0) return false;

        if (Math.Round(2.5) != 2.0 || Math.Round(3.5) != 4.0 || Math.Round(-2.5) != -2.0) return false;
        if (Math.Round(2.5, MidpointRounding.AwayFromZero) != 3.0) return false;
        if (Math.Round(2.7, MidpointRounding.ToZero) != 2.0) return false;
        if (Math.Round(-2.7, MidpointRounding.ToNegativeInfinity) != -3.0) return false;
        if (Math.Round(2.7, MidpointRounding.ToPositiveInfinity) != 3.0) return false;
        if (Math.Truncate(-1.75) != -1.0) return false;
        if (!NegativeZero(Math.Sin(-0.0)) || !NegativeZero(Math.Tan(-0.0)) || !NegativeZero(Math.Atan(-0.0))) return false;
        if (!NegativeZero(Math.Asin(-0.0)) || !NegativeZero(Math.Asinh(-0.0)) || !NegativeZero(Math.Sinh(-0.0))) return false;
        if (!NegativeZero(Math.Tanh(-0.0)) || !NegativeZero(Math.Cbrt(-0.0))) return false;
        if (!NegativeZero(Math.Floor(-0.0)) || !NegativeZero(Math.Ceiling(-0.0)) || !NegativeZero(Math.Truncate(-0.0))) return false;
        if (BitConverter.DoubleToInt64Bits(Math.Abs(-0.0)) != 0) return false;

        if (!double.IsNaN(Math.Sin(double.PositiveInfinity)) || !double.IsNaN(Math.Sqrt(-1.0))) return false;
        if (!double.IsNaN(Math.Acos(2.0)) || !double.IsNaN(Math.Atanh(2.0)) || !double.IsNaN(Math.Log(-1.0))) return false;
        if (!double.IsNaN(Math.FusedMultiplyAdd(double.NaN, 1.0, 0.0))) return false;
        if (!double.IsPositiveInfinity(Math.Cosh(double.NegativeInfinity))) return false;
        if (!double.IsPositiveInfinity(Math.Exp(double.PositiveInfinity)) || !double.IsNegativeInfinity(Math.Floor(double.NegativeInfinity))) return false;
        if (!double.IsNegativeInfinity(Math.Cbrt(double.NegativeInfinity)) || !double.IsPositiveInfinity(Math.Atanh(1.0))) return false;

        var (sin, cos) = Math.SinCos(-0.0);
        if (!NegativeZero(sin) || cos != 1.0) return false;
        return CheckDoubleModF();
    }

    private static bool CheckSingle()
    {
        if (MathF.Acos(1.0f) != 0.0f || MathF.Acosh(1.0f) != 0.0f) return false;
        if (MathF.Asin(0.0f) != 0.0f || MathF.Asinh(0.0f) != 0.0f) return false;
        if (MathF.Atan(0.0f) != 0.0f || MathF.Atanh(0.0f) != 0.0f) return false;
        if (MathF.Atan2(0.0f, 1.0f) != 0.0f || MathF.Cbrt(1.0f) != 1.0f) return false;
        if (MathF.Ceiling(1.25f) != 2.0f || MathF.Ceiling(-1.25f) != -1.0f) return false;
        if (MathF.Cos(0.0f) != 1.0f || MathF.Cosh(0.0f) != 1.0f || MathF.Exp(0.0f) != 1.0f) return false;
        if (MathF.Floor(1.75f) != 1.0f || MathF.Floor(-1.75f) != -2.0f) return false;
        if (MathF.FusedMultiplyAdd(2.0f, 4.0f, 1.0f) != 9.0f) return false;
        const float epsilon = 1.0f / 8192.0f;
        if (MathF.FusedMultiplyAdd(1.0f + epsilon, 1.0f - epsilon, -1.0f) != -1.0f / 67108864.0f) return false;
        if (MathF.Log(1.0f) != 0.0f || MathF.Log2(1.0f) != 0.0f || MathF.Log10(1.0f) != 0.0f) return false;
        if (MathF.Pow(2.0f, 10.0f) != 1024.0f || MathF.Sin(0.0f) != 0.0f || MathF.Sinh(0.0f) != 0.0f) return false;
        if (MathF.Sqrt(1.0f) != 1.0f || MathF.Tan(0.0f) != 0.0f || MathF.Tanh(0.0f) != 0.0f) return false;

        if (MathF.Round(2.5f) != 2.0f || MathF.Round(3.5f) != 4.0f || MathF.Round(-2.5f) != -2.0f) return false;
        if (MathF.Round(2.5f, MidpointRounding.AwayFromZero) != 3.0f) return false;
        if (MathF.Round(2.7f, MidpointRounding.ToZero) != 2.0f) return false;
        if (MathF.Round(-2.7f, MidpointRounding.ToNegativeInfinity) != -3.0f) return false;
        if (MathF.Round(2.7f, MidpointRounding.ToPositiveInfinity) != 3.0f) return false;
        if (MathF.Truncate(-1.75f) != -1.0f) return false;
        if (!NegativeZero(MathF.Sin(-0.0f)) || !NegativeZero(MathF.Tan(-0.0f)) || !NegativeZero(MathF.Atan(-0.0f))) return false;
        if (!NegativeZero(MathF.Asin(-0.0f)) || !NegativeZero(MathF.Asinh(-0.0f)) || !NegativeZero(MathF.Sinh(-0.0f))) return false;
        if (!NegativeZero(MathF.Tanh(-0.0f)) || !NegativeZero(MathF.Cbrt(-0.0f))) return false;
        if (!NegativeZero(MathF.Floor(-0.0f)) || !NegativeZero(MathF.Ceiling(-0.0f)) || !NegativeZero(MathF.Truncate(-0.0f))) return false;
        if (BitConverter.SingleToInt32Bits(MathF.Abs(-0.0f)) != 0) return false;

        if (!float.IsNaN(MathF.Sin(float.PositiveInfinity)) || !float.IsNaN(MathF.Sqrt(-1.0f))) return false;
        if (!float.IsNaN(MathF.Acos(2.0f)) || !float.IsNaN(MathF.Atanh(2.0f)) || !float.IsNaN(MathF.Log(-1.0f))) return false;
        if (!float.IsNaN(MathF.FusedMultiplyAdd(float.NaN, 1.0f, 0.0f))) return false;
        if (!float.IsPositiveInfinity(MathF.Cosh(float.NegativeInfinity))) return false;
        if (!float.IsPositiveInfinity(MathF.Exp(float.PositiveInfinity)) || !float.IsNegativeInfinity(MathF.Floor(float.NegativeInfinity))) return false;
        if (!float.IsNegativeInfinity(MathF.Cbrt(float.NegativeInfinity)) || !float.IsPositiveInfinity(MathF.Atanh(1.0f))) return false;

        var (sin, cos) = MathF.SinCos(-0.0f);
        if (!NegativeZero(sin) || cos != 1.0f) return false;
        return CheckSingleModF();
    }

    public static int Main()
    {
        if (!CheckDouble()) return 1;
        if (!CheckSingle()) return 2;
        Console.WriteLine("Math extern edge checks: OK");

        var stopwatch = Stopwatch.StartNew();
        var elapsed = stopwatch.Elapsed.TotalMilliseconds;
        var stopwatchIsSane = elapsed >= 0.0 && !double.IsNaN(elapsed) && !double.IsInfinity(elapsed);
        Console.WriteLine($"Stopwatch elapsed sanity: {stopwatchIsSane}");
        return stopwatchIsSane ? 42 : 3;
    }
}
