from app import BarcodeDecoder

def main():
    # Create an instance of BarcodeDecoder
    barcode_decoder = BarcodeDecoder()

    # Set the path to the image
    image_path = "your_image_path.jpg"

    # Decode the barcode
    barcode_data, barcode_type = barcode_decoder.decode_barcode(image_path)

    # Print the decoded barcode data and type
    if barcode_data:
        print("Barcode Data:", barcode_data)
        print("Barcode Type:", barcode_type)
    else:
        print("No barcode detected or decoded.")

if __name__ == "__main__":
    main()
