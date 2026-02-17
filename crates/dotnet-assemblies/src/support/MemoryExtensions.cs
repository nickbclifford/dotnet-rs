using System.Collections.Generic;
using System.Runtime.CompilerServices;
using DotnetRs;

namespace System
{
    [Stub(InPlaceOf = "System.MemoryExtensions")]
    public static class MemoryExtensions
    {
        [MethodImpl(MethodImplOptions.InternalCall)]
        public static extern bool SequenceEqual<T>(ReadOnlySpan<T> span, ReadOnlySpan<T> other);

        public static Span<T> AsSpan<T>(this T[] array) => new Span<T>(array);

        public static Span<T> AsSpan<T>(this T[] array, int start) => new Span<T>(array, start, array == null ? 0 : array.Length - start);

        public static Span<T> AsSpan<T>(this T[] array, int start, int length) => new Span<T>(array, start, length);

        public static ReadOnlySpan<char> AsSpan(this string text) => new ReadOnlySpan<char>(text.ToCharArray()); // Simplified for now

        public static ReadOnlySpan<char> AsSpan(this string text, int start) => new ReadOnlySpan<char>(text.ToCharArray(), start, text == null ? 0 : text.Length - start);

        public static ReadOnlySpan<char> AsSpan(this string text, int start, int length) => new ReadOnlySpan<char>(text.ToCharArray(), start, length);

        internal static bool SequenceEqualSlowPath<T>(ReadOnlySpan<T> span, ReadOnlySpan<T> other)
        {
            // Note: length equality and non-zero length are already checked by the intrinsic
            for (int i = 0; i < span.Length; i++)
            {
                if (!EqualityComparer<T>.Default.Equals(span[i], other[i]))
                    return false;
            }
            return true;
        }
    }
}
