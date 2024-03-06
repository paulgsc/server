import cv2
import numpy as np

class BarcodeDetector:
    def __init__(self):
        self.barcode_detector = cv2.barcode_BarcodeDetector()

    def preprocess_image(self, image_path):
        """
        Preprocess the input image for better barcode detection.
        """
        im = cv2.imread(image_path, cv2.IMREAD_GRAYSCALE)

        # Apply Gaussian blur
        blur = cv2.GaussianBlur(im, (5, 5), 0)

        # Apply thresholding
        ret, bw_im = cv2.threshold(blur, 0, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU)

        return bw_im

    def detect_barcodes(self, preprocessed_image):
        """
        Detect barcodes in the preprocessed image.
        """
        barcodes = self.barcode_detector.detect(preprocessed_image)
        return barcodes

    def extract_barcode(self, image, barcode):
        """
        Extract the barcode region from the image.
        """
        x, y, w, h = barcode.rect
        barcode_image = image[y:y+h, x:x+w]
        return barcode_image

    def show_preprocessed_image(self, preprocessed_image):
        """
        Display the preprocessed image.
        """
        # Display the preprocessed image
        cv2.imshow("Preprocessed Image", preprocessed_image)
        cv2.waitKey(0)
        cv2.destroyAllWindows()

class BarcodeDecoder(BarcodeDetector):
    def __init__(self):
        super().__init__()
        self.barcode_types = [cv2.IMREAD_UNCHANGED, cv2.IMREAD_GRAYSCALE, cv2.IMREAD_COLOR]

    def decode_barcode(self, image_path):
        image = cv2.imread(image_path)
        preprocessed_image = self.preprocess_image(image_path)
        self.show_preprocessed_image(preprocessed_image)
        barcodes = decode(preprocessed_image)

        if barcodes:
            for barcode in barcodes:
                barcode_data = barcode.data.decode("utf-8")
                barcode_type = barcode.type
                return barcode_data, barcode_type

        return None, None
