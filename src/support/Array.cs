using System.Collections;

namespace DotnetRs;

[Stub(InPlaceOf = "System.Array")]
public class Array : ICloneable, IList, IStructuralComparable, IStructuralEquatable
{
    public int Length => GetLength(0);

    public long LongLength => GetLongLength(0);

    public nuint NativeLength => (nuint)Length;

    public int Rank => GetRank();

    public virtual int GetLength(int dimension) => throw new NotImplementedException();

    public long GetLongLength(int dimension) => GetLength(dimension);

    public virtual int GetRank() => throw new NotImplementedException();

    public virtual object? GetValue(params int[] indices) => throw new NotImplementedException();

    public virtual void SetValue(object? value, params int[] indices) => throw new NotImplementedException();

    public object Clone() => MemberwiseClone();

    // IList implementation
    public bool IsFixedSize => true;
    public bool IsReadOnly => false;
    public bool IsSynchronized => false;
    public object SyncRoot => this;
    public int Count => Length;

    object? IList.this[int index]
    {
        get => GetValue(index);
        set => SetValue(value, index);
    }

    int IList.Add(object? value) => throw new NotSupportedException();
    void IList.Clear() => throw new NotSupportedException();
    bool IList.Contains(object? value) => IndexOf((Array)this, value) >= 0;
    int IList.IndexOf(object? value) => IndexOf((Array)this, value);
    void IList.Insert(int index, object? value) => throw new NotSupportedException();
    void IList.Remove(object? value) => throw new NotSupportedException();
    void IList.RemoveAt(int index) => throw new NotSupportedException();

    public IEnumerator GetEnumerator() => new ArrayEnumerator(this);

    public void CopyTo(System.Array array, int index)
    {
        if (array == null) throw new ArgumentNullException(nameof(array));
        if (Rank != 1) throw new RankException();
        if (index < 0) throw new ArgumentOutOfRangeException(nameof(index));
        if (index + Length > array.Length) throw new ArgumentException();

        for (int i = 0; i < Length; i++)
        {
            array.SetValue(GetValue(i), index + i);
        }
    }

    // IStructuralComparable / IStructuralEquatable stubs
    int IStructuralComparable.CompareTo(object? other, IComparer comparer) => throw new NotImplementedException();
    bool IStructuralEquatable.Equals(object? other, IEqualityComparer comparer) => throw new NotImplementedException();
    int IStructuralEquatable.GetHashCode(IEqualityComparer comparer) => throw new NotImplementedException();

    // Static helper methods
    public static int IndexOf(Array array, object? value)
    {
        if (array == null) throw new ArgumentNullException(nameof(array));
        return IndexOf(array, value, 0, array.Length);
    }

    public static int IndexOf(Array array, object? value, int startIndex)
    {
        if (array == null) throw new ArgumentNullException(nameof(array));
        return IndexOf(array, value, startIndex, array.Length - startIndex);
    }

    public static int IndexOf(Array array, object? value, int startIndex, int count)
    {
        if (array == null) throw new ArgumentNullException(nameof(array));
        if (startIndex < 0 || startIndex > array.Length) throw new ArgumentOutOfRangeException(nameof(startIndex));
        if (count < 0 || startIndex + count > array.Length) throw new ArgumentOutOfRangeException(nameof(count));

        for (int i = startIndex; i < startIndex + count; i++)
        {
            var item = array.GetValue(i);
            if (item == null && value == null) return i;
            if (item != null && item.Equals(value)) return i;
        }
        return -1;
    }

    public static int LastIndexOf(Array array, object? value)
    {
        if (array == null) throw new ArgumentNullException(nameof(array));
        return LastIndexOf(array, value, array.Length - 1, array.Length);
    }

    public static int LastIndexOf(Array array, object? value, int startIndex)
    {
        if (array == null) throw new ArgumentNullException(nameof(array));
        return LastIndexOf(array, value, startIndex, startIndex + 1);
    }

    public static int LastIndexOf(Array array, object? value, int startIndex, int count)
    {
        if (array == null) throw new ArgumentNullException(nameof(array));
        if (array.Length == 0) return -1;
        if (startIndex < 0 || startIndex >= array.Length) throw new ArgumentOutOfRangeException(nameof(startIndex));
        if (count < 0 || startIndex - count + 1 < 0) throw new ArgumentOutOfRangeException(nameof(count));

        for (int i = startIndex; i > startIndex - count; i--)
        {
            var item = array.GetValue(i);
            if (item == null && value == null) return i;
            if (item != null && item.Equals(value)) return i;
        }
        return -1;
    }

    public static void Copy(Array sourceArray, Array destinationArray, int length)
    {
        Copy(sourceArray, 0, destinationArray, 0, length);
    }

    public static void Copy(Array sourceArray, int sourceIndex, Array destinationArray, int destinationIndex, int length)
    {
        if (sourceArray == null) throw new ArgumentNullException(nameof(sourceArray));
        if (destinationArray == null) throw new ArgumentNullException(nameof(destinationArray));
        if (length < 0) throw new ArgumentOutOfRangeException(nameof(length));
        if (sourceIndex < 0) throw new ArgumentOutOfRangeException(nameof(sourceIndex));
        if (destinationIndex < 0) throw new ArgumentOutOfRangeException(nameof(destinationIndex));
        if (sourceIndex + length > sourceArray.Length) throw new ArgumentException();
        if (destinationIndex + length > destinationArray.Length) throw new ArgumentException();

        for (int i = 0; i < length; i++)
        {
            destinationArray.SetValue(sourceArray.GetValue(sourceIndex + i), destinationIndex + i);
        }
    }

    public static void Clear(Array array)
    {
        if (array == null) throw new ArgumentNullException(nameof(array));
        Clear(array, 0, array.Length);
    }

    public static void Clear(Array array, int index, int length)
    {
        if (array == null) throw new ArgumentNullException(nameof(array));
        if (index < 0) throw new ArgumentOutOfRangeException(nameof(index));
        if (length < 0) throw new ArgumentOutOfRangeException(nameof(length));
        if (index + length > array.Length) throw new ArgumentException();

        for (int i = index; i < index + length; i++)
        {
            array.SetValue(null, i);
        }
    }

    public static T[] Empty<T>() => new T[0];

    private class ArrayEnumerator : IEnumerator
    {
        private readonly Array _array;
        private int _index = -1;

        public ArrayEnumerator(Array array)
        {
            _array = array;
        }

        public object? Current => _array.GetValue(_index);

        public bool MoveNext()
        {
            if (_index < _array.Length - 1)
            {
                _index++;
                return true;
            }
            return false;
        }

        public void Reset() => _index = -1;
    }
}
